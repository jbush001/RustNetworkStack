//
// Copyright 2024 Jeff Bush
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

use crate::buf;
use crate::ipv4;
use crate::netif;
use crate::timer;
use crate::util;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// Each socket is uniquely identified by the tuple of remote_ip/remote_port/local_port
type SocketKey = (util::IPv4Addr, u16, u16);
const EPHEMERAL_PORT_BASE: u16 = 49152;
const TCP_MTU: usize = 1500;
const RETRANSMIT_INTERVAL: u32 = 1000; // HACK: this should back off
const MAX_ACK_DELAY: u32 = 500; // ms
const MAX_DELAYED_ACKS: u32 = 5;
const RESPONSE_TIMEOUT: u32 = 3000; // ms
const TIME_WAIT_TIMEOUT: u32 = 5000; // ms

const MAX_RECEIVE_WINDOW: u16 = 0xffff;
const MAX_RETRIES: u32 = 5; // For connection management

#[derive(Debug)]
enum TCPState {
    Closed,
    SynSent,
    Established,
    CloseWait,
    LastAck,
    FinWait1,
    FinWait2,
    Closing,
    TimeWait,
}

const FLAG_FIN: u8 = 1;
const FLAG_SYN: u8 = 2;
const FLAG_RST: u8 = 4;
const FLAG_PSH: u8 = 8;
const FLAG_ACK: u8 = 16;

pub struct TCPSocket {
    remote_ip: util::IPv4Addr,
    remote_port: u16,
    local_port: u16,
    state: TCPState,

    // Receive
    receive_queue: buf::NetBuffer,
    reassembler: TCPReassembler,
    delayed_ack_timer_id: i32,
    num_delayed_acks: u32,
    highest_seq_received: u32,

    // Transmit
    next_transmit_seq: u32,
    retransmit_queue: buf::NetBuffer,
    transmit_window_max: u32, // Highest sequence we can transmit
    retransmit_timer_id: i32,
    response_timer_id: i32,
    request_retry_count: u32,
}

pub struct TCPReassembler {
    next_sequence: u32,
    out_of_order: Vec<(u32, buf::NetBuffer)>,
}

type SocketReference = Arc<(Mutex<TCPSocket>, Condvar)>;
type PortMap = HashMap<SocketKey, SocketReference>;

lazy_static! {
    static ref PORT_MAP: Mutex<PortMap> = Mutex::new(HashMap::new());
}

/// Generate a random ephemeral port that doesn't conflict with any open sockets.
fn get_ephemeral_port(
    guard: &mut MutexGuard<PortMap>,
    remote_ip: util::IPv4Addr,
    remote_port: u16,
) -> u16 {
    loop {
        const RANGE: u16 = 0xffff - EPHEMERAL_PORT_BASE;
        let port = EPHEMERAL_PORT_BASE + (rand::random::<u16>() % RANGE);
        if !guard.contains_key(&(remote_ip, remote_port, port)) {
            return port;
        }
    }
}

pub fn tcp_open(
    remote_ip: util::IPv4Addr,
    remote_port: u16,
) -> Result<SocketReference, &'static str> {

    let mut portmap_guard = PORT_MAP.lock().unwrap();
    let local_port = get_ephemeral_port(&mut portmap_guard, remote_ip, remote_port);
    let socket = Arc::new((
        Mutex::new(TCPSocket::new(remote_ip, remote_port, local_port)),
        Condvar::new(),
    ));

    portmap_guard.insert((remote_ip, remote_port, local_port), socket.clone());
    drop(portmap_guard);

    let (mutex, cond) = &*socket;
    let mut guard = mutex.lock().unwrap();
    guard.set_state(TCPState::SynSent);

    guard.send_packet(buf::NetBuffer::new(), FLAG_SYN);
    set_response_timer(&mut guard, socket.clone());

    // Wait until this is connected
    while !matches!(guard.state, TCPState::Established) {
        guard = cond.wait(guard).unwrap();
        if matches!(guard.state, TCPState::Closed) {
            return Err("Connection failed");
        }
    }

    std::mem::drop(guard);

    Ok(socket)
}

pub fn tcp_close(socket: &mut SocketReference) {
    let (mutex, _cond) = &**socket;
    let mut guard = mutex.lock().unwrap();

    println!("{} tcp_close: state {:?}", guard, guard.state);
    match guard.state {
        TCPState::Established => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_FIN | FLAG_ACK);
            set_response_timer(&mut guard, socket.clone());
            guard.set_state(TCPState::FinWait1);
        }

        TCPState::CloseWait => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_FIN | FLAG_ACK);
            set_response_timer(&mut guard, socket.clone());
            guard.set_state(TCPState::LastAck);
        }

        _ => {}
    }
}

pub fn tcp_read(socket: &mut SocketReference, data: &mut [u8]) -> i32 {
    let (mutex, cond) = &**socket;
    let mut guard = mutex.lock().unwrap();

    loop {
        if !matches!(guard.state, TCPState::Established) && guard.receive_queue.len() == 0 {
            return -1;
        }

        if guard.receive_queue.len() > 0 {
            let got = guard.receive_queue.copy_to_slice(data);
            guard.receive_queue.trim_head(got);
            return got as i32;
        }

        guard = cond.wait(guard).unwrap();
    }
}

pub fn tcp_write(socket: &mut SocketReference, data: &[u8]) -> i32 {
    assert!(data.len() < TCP_MTU); // XXX Fix this at some point

    let (mutex, _cond) = &**socket;
    let mut guard = mutex.lock().unwrap();

    if matches!(guard.state, TCPState::Closed) {
        return -1;
    }

    let mut packet = buf::NetBuffer::new();
    packet.append_from_slice(data);
    guard.send_packet(packet, FLAG_ACK | FLAG_PSH);

    guard.next_transmit_seq = guard.next_transmit_seq.wrapping_add(data.len() as u32);
    guard.retransmit_queue.append_from_slice(data);
    if guard.retransmit_timer_id == -1 {
        let socket_arc = socket.clone();
        guard.retransmit_timer_id = timer::set_timer(RETRANSMIT_INTERVAL, move || {
            retransmit(socket_arc);
        });
    }

    data.len() as i32
}

fn retransmit(socket: SocketReference) {
    let (mutex, _cond) = &*socket;
    let mut guard = mutex.lock().unwrap();

    if matches!(guard.state, TCPState::Closed) {
        return;
    }

    if guard.retransmit_queue.len() > 0 {
        println!("Retransmitting sequence {}", guard.next_transmit_seq);
        let mut packet = buf::NetBuffer::new();
        packet.append_from_buffer(&guard.retransmit_queue, TCP_MTU);
        util::print_binary(packet.header());
        guard.send_packet(packet, FLAG_ACK | FLAG_PSH);
        let socket_clone = socket.clone();
        guard.retransmit_timer_id = timer::set_timer(RETRANSMIT_INTERVAL, move || {
            retransmit(socket_clone);
        });
    }
}

fn flags_to_str(flags: u8) -> String {
    let mut result = String::new();
    if flags & FLAG_FIN != 0 {
        result.push('F');
    }

    if flags & FLAG_SYN != 0 {
        result.push('S');
    }

    if flags & FLAG_RST != 0 {
        result.push('R');
    }

    if flags & FLAG_PSH != 0 {
        result.push('P');
    }

    if flags & FLAG_ACK != 0 {
        result.push('A');
    }

    result
}

impl TCPSocket {
    fn new(remote_ip: util::IPv4Addr, remote_port: u16, local_port: u16) -> TCPSocket {
        TCPSocket {
            remote_ip,
            remote_port,
            local_port,
            next_transmit_seq: rand::random::<u32>(),
            transmit_window_max: 0,
            state: TCPState::Closed,
            receive_queue: buf::NetBuffer::new(),
            reassembler: TCPReassembler::new(),
            delayed_ack_timer_id: -1,
            num_delayed_acks: 0,
            retransmit_queue: buf::NetBuffer::new(),
            retransmit_timer_id: -1,
            response_timer_id: -1,
            request_retry_count: 0,
            highest_seq_received: 0,
        }
    }

    fn send_packet(&mut self, packet: buf::NetBuffer, flags: u8) {
        let receive_window = MAX_RECEIVE_WINDOW - self.receive_queue.len() as u16;

        // We need to acknowledge the FIN packet, which consumes a sequence
        // number. But we should only do this if we have received all other outstanding
        // data.
        let ack_seq = self.reassembler.get_next_expect()
            + if matches!(
                self.state,
                TCPState::FinWait1 | TCPState::FinWait2 | TCPState::Closing | TCPState::CloseWait
            ) && self.highest_seq_received == self.reassembler.get_next_expect()
            {
                1
            } else {
                0
            };

        println!(
            "{}: send_packet: flags {} seq {} ack {} window {}",
            self,
            flags_to_str(flags),
            self.next_transmit_seq,
            ack_seq,
            receive_window
        );

        tcp_output(
            packet,
            self.local_port,
            self.remote_ip,
            self.remote_port,
            self.next_transmit_seq,
            ack_seq,
            flags,
            receive_window,
        );
    }

    fn set_state(&mut self, new_state: TCPState) {
        println!(
            "{}: Change state from {:?} to {:?}",
            self, self.state, new_state
        );
        self.state = new_state;
        self.request_retry_count = 0;
    }
}

impl Display for TCPSocket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "localhost:{} {}:{}",
            self.local_port, self.remote_ip, self.remote_port
        )
    }
}

impl TCPReassembler {
    fn new() -> TCPReassembler {
        TCPReassembler {
            next_sequence: 0,
            out_of_order: Vec::new(),
        }
    }

    fn set_next_expect(&mut self, seq_num: u32) {
        self.next_sequence = seq_num;
    }

    fn add_packet(&mut self, mut packet: buf::NetBuffer, seq_num: u32) -> Option<buf::NetBuffer> {
        if seq_num == self.next_sequence {
            self.next_sequence = self.next_sequence.wrapping_add(packet.len() as u32);

            // Check if any of the out-of-order packets can now be reassembled.
            let mut i = 0;
            while i < self.out_of_order.len() {
                if util::seq_gt(seq_num, self.out_of_order[i].0) {
                    // Remove packets before window.
                    self.out_of_order.remove(i);
                } else if self.out_of_order[i].0 == self.next_sequence {
                    let (_, ooo_packet) = self.out_of_order.remove(i);
                    self.next_sequence = self.next_sequence.wrapping_add(ooo_packet.len() as u32);
                    packet.append_buffer(ooo_packet);
                    i = 0;
                } else {
                    i += 1;
                }
            }

            Some(packet)
        } else {
            // Note that this doesn't bother to order these or anything. I assume
            // this case is infrequent enough that any optimization would be
            // lost in the noise.
            self.out_of_order.push((seq_num, packet));
            None
        }
    }

    fn get_next_expect(&self) -> u32 {
        self.next_sequence
    }
}

const TCP_HEADER_LEN: usize = 20;

//
//    0               1               2               3
//    +-------------------------------+-------------------------------+
//  0 |         Source Port           |          Dest Port            |
//    +-------------------------------+-------------------------------+
//  4 |                        Sequence Number                        |
//    +-------------------------------+-------------------------------+
//  8 |                           Ack Number                          |
//    +-------+-------+---------------+-------------------------------+
// 12 |  Offs | Rsvd  |   CEUAPRSF    |            Window             |
//    +-------+-------+---------------+-------------------------------+
// 16 |          Checksum             |        Urgent Pointer         |
//    +-------------------------------+-------------------------------+
// 20 |                           [Options]                           |
//    +---------------------------------------------------------------+
//

pub fn tcp_input(mut packet: buf::NetBuffer, source_ip: util::IPv4Addr) {
    let header = packet.header_mut();
    let source_port = util::get_be16(&header[0..2]);
    let dest_port = util::get_be16(&header[2..4]);
    let seq_num = util::get_be32(&header[4..8]);
    let ack_num = util::get_be32(&header[8..12]);
    let header_size = ((header[12] >> 4) * 4) as usize;
    let remote_window_size = util::get_be16(&header[14..16]);
    let flags = header[13];

    packet.trim_head(header_size);

    // Lookup socket
    let mut port_map_guard = PORT_MAP.lock().unwrap();
    let pm_entry = port_map_guard.get_mut(&(source_ip, source_port, dest_port));
    if pm_entry.is_none() {
        let response = buf::NetBuffer::new();
        tcp_output(
            response,
            dest_port,
            source_ip,
            source_port,
            1,           // Sequence number
            seq_num + 1, // Acknowledge sequence from host.
            FLAG_RST | FLAG_ACK,
            0,
        );

        return;
    }

    let socket = pm_entry
        .expect("just checked if pm_entry is none above")
        .clone();
    let (mutex, cond) = &*socket;
    let mut guard = mutex.lock().unwrap();

    println!(
        "{}: tcp_input: flags {} seq {} ack {} window {} ({} bytes of data)",
        guard,
        flags_to_str(flags),
        seq_num,
        ack_num,
        remote_window_size,
        packet.len()
    );
    if flags & FLAG_ACK != 0 {
        let expected = if matches!(guard.state, TCPState::Established) {
            guard.next_transmit_seq
        } else {
            guard.next_transmit_seq.wrapping_add(1)
        };

        if ack_num != expected {
            println!(
                "{}: Unexpected ack {} expected {}",
                guard, ack_num, expected
            );
        }
    }

    if guard.response_timer_id != -1 {
        timer::cancel_timer(guard.response_timer_id);
        guard.response_timer_id = -1;
    }

    if (flags & FLAG_RST) != 0 {
        println!("{}: Connection reset", guard);
        guard.set_state(TCPState::Closed);
        cond.notify_all();
        return;
    }

    if packet.len() > 0 {
        // Handle received data
        guard.highest_seq_received = std::cmp::max(
            guard.highest_seq_received,
            seq_num.wrapping_add(packet.len() as u32),
        );
        let got = guard.reassembler.add_packet(packet, seq_num);
        if let Some(socketdata) = got {
            guard.receive_queue.append_buffer(socketdata);
            cond.notify_all();
        }

        if matches!(guard.state, TCPState::Established) {
            guard.num_delayed_acks += 1;
            if guard.num_delayed_acks >= MAX_DELAYED_ACKS || (flags & FLAG_FIN) != 0 {
                println!(
                    "{}: Sending immediate ack, num_delayed_acks={}",
                    guard, guard.num_delayed_acks
                );
                guard.num_delayed_acks = 0;
                if guard.delayed_ack_timer_id != -1 {
                    timer::cancel_timer(guard.delayed_ack_timer_id);
                    guard.delayed_ack_timer_id = -1;
                }

                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
            } else if guard.delayed_ack_timer_id == -1 {
                println!("{}: Starting delayed ack timer", guard);
                let socket_clone = socket.clone();
                guard.delayed_ack_timer_id = timer::set_timer(MAX_ACK_DELAY, move || {
                    let (mutex, _cond) = &*socket_clone;
                    let mut guard = mutex.lock().unwrap();

                    if matches!(guard.state, TCPState::Closed) {
                        return;
                    }

                    println!("{}: Sending delayed ack", guard);
                    guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                    guard.delayed_ack_timer_id = -1;
                    guard.num_delayed_acks = 0;
                });
            } else {
                println!(
                    "{}: Deferring ack, count is now {}",
                    guard, guard.num_delayed_acks
                );
            }
        } else {
            if guard.delayed_ack_timer_id != -1 {
                timer::cancel_timer(guard.delayed_ack_timer_id);
                guard.delayed_ack_timer_id = -1;
            }

            guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
        }
    }

    match guard.state {
        TCPState::SynSent => {
            if (flags & FLAG_ACK) != 0 {
                guard.set_state(TCPState::Established);
                guard.reassembler.set_next_expect(seq_num + 1);

                // The SYN consumes a sequence number.
                guard.next_transmit_seq = guard.next_transmit_seq.wrapping_add(1);

                // Send ack to complete handshake
                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                set_response_timer(&mut guard, socket.clone());

                // Wake up thread waiting in connect
                cond.notify_all();
            }
        }

        TCPState::Established => {
            if (flags & FLAG_FIN) != 0 {
                guard.set_state(TCPState::CloseWait);

                // Ack will be sent below. FIN packets can contain data.
                cond.notify_all();
            }

            if (flags & FLAG_ACK) != 0 {
                let oldest_unacked = guard
                    .next_transmit_seq
                    .wrapping_sub(guard.retransmit_queue.len() as u32);
                if util::seq_gt(ack_num, oldest_unacked) {
                    let trim = ack_num.wrapping_sub(oldest_unacked) as usize;
                    guard.retransmit_queue.trim_head(trim);
                    println!(
                        "{}: Trimming {} acked bytes from retransmit queue, size is now {}",
                        guard,
                        trim,
                        guard.retransmit_queue.len()
                    );

                    if guard.retransmit_queue.len() == 0 {
                        timer::cancel_timer(guard.retransmit_timer_id);
                        guard.retransmit_timer_id = -1;
                    }
                }

                guard.transmit_window_max = ack_num.wrapping_add(remote_window_size as u32);
            }
        }

        TCPState::LastAck => {
            if (flags & FLAG_ACK) != 0 {
                guard.set_state(TCPState::Closed);
            }
        }

        TCPState::FinWait1 => {
            if (flags & FLAG_ACK != 0)
                && (flags & FLAG_FIN != 0)
                && ack_num == guard.next_transmit_seq.wrapping_add(1)
            {
                guard.set_state(TCPState::TimeWait);
                let socket_clone = socket.clone();
                timer::set_timer(TIME_WAIT_TIMEOUT, move || {
                    time_wait_timeout(socket_clone);
                });
            } else if (flags & FLAG_FIN) != 0 {
                guard.set_state(TCPState::Closing);
                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                set_response_timer(&mut guard, socket.clone());
            } else if (flags & FLAG_ACK) != 0 && ack_num == guard.next_transmit_seq.wrapping_add(1)
            {
                guard.set_state(TCPState::FinWait2);
            }
        }

        TCPState::FinWait2 => {
            if (flags & FLAG_FIN) != 0 {
                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                set_response_timer(&mut guard, socket.clone());
                guard.set_state(TCPState::TimeWait);
                let socket_clone = socket.clone();
                timer::set_timer(TIME_WAIT_TIMEOUT, move || {
                    time_wait_timeout(socket_clone);
                });
            }
        }

        TCPState::Closing => {
            if (flags & FLAG_ACK) != 0 {
                guard.set_state(TCPState::TimeWait);
                let socket_clone = socket.clone();
                timer::set_timer(TIME_WAIT_TIMEOUT, move || {
                    time_wait_timeout(socket_clone);
                });
            }
        }

        _ => {
            println!("{}: Received packet in state: {:?}", guard, guard.state);
        }
    }
}

pub fn tcp_output(
    mut packet: buf::NetBuffer,
    source_port: u16,
    dest_ip: util::IPv4Addr,
    dest_port: u16,
    seq_num: u32,
    ack_num: u32,
    flags: u8,
    window: u16,
) {
    packet.alloc_header(TCP_HEADER_LEN);
    let length = packet.len() as u16;
    {
        let header = packet.header_mut();
        util::set_be16(&mut header[0..2], source_port);
        util::set_be16(&mut header[2..4], dest_port);
        util::set_be32(&mut header[4..8], seq_num);
        util::set_be32(&mut header[8..12], ack_num);
        header[12] = ((TCP_HEADER_LEN / 4) << 4) as u8; // Data offset
        header[13] = flags;
        util::set_be16(&mut header[14..16], window);
    }

    // Compute checksum
    // First need to create a pseudo header
    let mut pseudo_header = [0u8; 12];
    netif::get_ipaddr().copy_to(&mut pseudo_header[0..4]);
    dest_ip.copy_to(&mut pseudo_header[4..8]);
    pseudo_header[8] = 0; // Reserved
    pseudo_header[9] = ipv4::PROTO_TCP; // Protocol
    util::set_be16(&mut pseudo_header[10..12], length); // TCP length (header + data)

    let ph_sum = util::compute_ones_comp(0, &pseudo_header);
    let checksum = util::compute_buffer_ones_comp(ph_sum, &packet) ^ 0xffff;

    let header = packet.header_mut();
    util::set_be16(&mut header[16..18], checksum);

    ipv4::ip_output(packet, ipv4::PROTO_TCP, dest_ip);
}

fn set_response_timer(guard: &mut MutexGuard<TCPSocket>, socket: SocketReference) {
    if guard.response_timer_id != -1 {
        timer::cancel_timer(guard.response_timer_id);
    }

    let socket_clone = socket.clone();
    guard.response_timer_id = timer::set_timer(RESPONSE_TIMEOUT, move || {
        response_timeout(socket_clone);
    });
}

fn response_timeout(socket: SocketReference) {
    let (mutex, cond) = &*socket;
    let mut guard = mutex.lock().unwrap();

    if guard.request_retry_count >= MAX_RETRIES {
        println!(
            "{}: Too many response timeouts in state {:?}, closing",
            guard, guard.state
        );
        guard.set_state(TCPState::Closed);
        cond.notify_all();
        return;
    }

    println!("{}: Response timeout state {:?}", guard, guard.state);
    match guard.state {
        TCPState::Closed | TCPState::Established => {
            // This can occur if the timer fires as the connection state
            // transitions.
        }

        TCPState::SynSent => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_SYN);
            set_response_timer(&mut guard, socket.clone());
        }

        TCPState::FinWait1 | TCPState::LastAck => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_FIN);
            set_response_timer(&mut guard, socket.clone());
        }

        TCPState::Closing | TCPState::CloseWait => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
            set_response_timer(&mut guard, socket.clone());
        }

        _ => {
            // This would indicate a bug: we set a timer where we shoudn't have.
            panic!(
                "{}: unexpected state in response_timeout: {:?}",
                guard, guard.state
            );
        }
    }

    set_response_timer(&mut guard, socket.clone());
}

fn time_wait_timeout(socket: SocketReference) {
    let (mutex, _cond) = &*socket;
    let mut guard = mutex.lock().unwrap();

    guard.set_state(TCPState::Closed);
    let remote_ip = guard.remote_ip;
    let remote_port = guard.remote_port;
    let local_port = guard.local_port;
    drop(guard); // Unlock to avoid deadlock
    PORT_MAP
        .lock()
        .unwrap()
        .remove(&(remote_ip, remote_port, local_port));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reassemble_inorder() {
        // Happy path: we get a packet, it is in order
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1234);
        let mut packet = buf::NetBuffer::new();
        packet.append_from_slice(b"hello");
        let result = reassembler.add_packet(packet, 1234);
        assert!(result.is_some());
        let new_packet = result.as_ref().unwrap();
        assert_eq!(reassembler.get_next_expect(), 1239);

        assert_eq!(new_packet.len(), 5);
        let mut data = [0u8; 5];
        let got = new_packet.copy_to_slice(&mut data);
        assert_eq!(got, 5);
    }

    #[test]
    fn test_reassemble_ooo() {
        // Two packets received out of order.
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1000);

        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 100]);

        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 100]);

        let result = reassembler.add_packet(packet2, 1100);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let result = reassembler.add_packet(packet1, 1000);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 1200);

        let new_packet = result.as_ref().unwrap();
        assert_eq!(new_packet.len(), 200);

        let mut data = [0u8; 200];
        new_packet.copy_to_slice(&mut data);
        assert!(data[0] == 1);
        assert!(data[99] == 1);
        assert!(data[100] == 2);
        assert!(data[199] == 2);
    }

    #[test]
    fn test_reassemble_stale1() {
        // Packet is received before sequence
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1000);

        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 100]);

        let result = reassembler.add_packet(packet1, 900);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 100]);
        let result = reassembler.add_packet(packet2, 1000);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 1100);

        assert_eq!(reassembler.out_of_order.len(), 0);
    }

    #[test]
    fn test_reassemble_stale2() {
        // Packet is received before sequence. We also have an out of order
        // segment that will be left in the reassembler.
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1000);

        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 100]);
        let result = reassembler.add_packet(packet1, 1200);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 100]);
        let result = reassembler.add_packet(packet2, 900);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let mut packet3 = buf::NetBuffer::new();
        packet3.append_from_slice(&[3; 100]);
        let result = reassembler.add_packet(packet3, 1000);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 1100);

        // Check output
        let new_packet = result.as_ref().unwrap();
        assert_eq!(new_packet.len(), 100);
        let mut data = [0u8; 100];
        new_packet.copy_to_slice(&mut data);
        assert!(data[0] == 3);
        assert!(data[99] == 3);

        assert_eq!(reassembler.out_of_order.len(), 1);
    }

    #[test]
    fn test_reassemble_wrap() {
        // Check wrapping case for sequence numbers
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(0xffffff00);

        // Packet before window. This should be removed.
        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 0x100]);
        let result = reassembler.add_packet(packet1, 0xfffffe00);
        assert!(result.is_none());

        // Fill window, wrap around
        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 0x200]);
        let result = reassembler.add_packet(packet2, 0xffffff00);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 0x100);

        let new_packet = result.as_ref().unwrap();
        assert_eq!(new_packet.len(), 0x200);
        let mut data = [0u8; 0x200];
        new_packet.copy_to_slice(&mut data);
        assert!(data[0] == 2);
        assert!(data[199] == 2);

        assert_eq!(reassembler.out_of_order.len(), 0);
    }

    #[test]
    fn test_reassemble_reorder_wrap() {
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(0xfffffe00);

        // This packet will cause a wrap when it's reassembled.
        // Ensure we are incrementing the sequence number correctly
        // in the case.
        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 0x200]);
        let result = reassembler.add_packet(packet1, 0xffffff00);
        assert!(result.is_none());

        // This packet will be in order.
        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 0x100]);
        let result = reassembler.add_packet(packet2, 0xfffffe00);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 0x100);
    }

    #[test]
    fn test_reassemble_multiple() {
        // Multiple packets get reassembled in one pass.
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1000);

        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 100]);

        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 100]);

        let mut packet3 = buf::NetBuffer::new();
        packet3.append_from_slice(&[3; 100]);

        let result = reassembler.add_packet(packet2, 1100);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let result = reassembler.add_packet(packet3, 1200);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let result = reassembler.add_packet(packet1, 1000);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 1300);

        let new_packet = result.as_ref().unwrap();
        assert_eq!(new_packet.len(), 300);

        let mut data = [0u8; 300];
        new_packet.copy_to_slice(&mut data);
        assert!(data[0] == 1);
        assert!(data[99] == 1);
        assert!(data[100] == 2);
        assert!(data[199] == 2);
        assert!(data[200] == 3);
        assert!(data[299] == 3);
    }

    #[test]
    fn test_reassemble_overlap1() {
        // It's possible a packet is not in order but overlaps
        // the current space. We will just drop it.

        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1000);

        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 100]);

        let result = reassembler.add_packet(packet2, 1100);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let mut packet1_prime = buf::NetBuffer::new();
        packet1_prime.append_from_slice(&[3; 150]);
        let result = reassembler.add_packet(packet1_prime, 1000);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 1150);

        let new_packet = result.as_ref().unwrap();
        assert_eq!(new_packet.len(), 150);

        let mut data = [0u8; 150];
        new_packet.copy_to_slice(&mut data);
        assert!(data[0] == 3);
        assert!(data[99] == 3);
        assert!(data[100] == 3);
        assert!(data[149] == 3);

        // Ensure the previous one was removed.
        assert_eq!(reassembler.out_of_order.len(), 1);
    }

    #[test]
    fn test_reassemble_overlap2() {
        // Another overlap case, but the overlapping packet was received
        // out of order.
        let mut reassembler = TCPReassembler::new();
        reassembler.set_next_expect(1000);

        let mut packet3 = buf::NetBuffer::new();
        packet3.append_from_slice(&[3; 100]);
        let result = reassembler.add_packet(packet3, 1200);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        let mut packet2 = buf::NetBuffer::new();
        packet2.append_from_slice(&[2; 150]); // Note this overlaps packet 3
        let result = reassembler.add_packet(packet2, 1100);
        assert!(result.is_none());
        assert_eq!(reassembler.get_next_expect(), 1000);

        // Now packet 1 comes in and completes. Packet 3 will be dropped.
        let mut packet1 = buf::NetBuffer::new();
        packet1.append_from_slice(&[1; 100]);
        let result = reassembler.add_packet(packet1, 1000);
        assert!(result.is_some());
        assert_eq!(reassembler.get_next_expect(), 1250);

        let new_packet = result.as_ref().unwrap();
        assert_eq!(new_packet.len(), 250);

        let mut data = [0u8; 250];
        new_packet.copy_to_slice(&mut data);
        assert!(data[0] == 1);
        assert!(data[99] == 1);
        assert!(data[100] == 2);
        assert!(data[249] == 2);

        // Ensure the previous one was removed.
        assert_eq!(reassembler.out_of_order.len(), 1);
    }
}
