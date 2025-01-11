//
// Copyright 2024-2025 Jeff Bush
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

// Transmission Control Protocol, as described in RFC 9293

use crate::buf;
use crate::ip;
use crate::netif;
use crate::timer;
use crate::util;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, LazyLock};

const EPHEMERAL_PORT_BASE: u16 = 49152;
const RETRANSMIT_INTERVAL: u32 = 1000; // HACK: this should back off
const MAX_ACK_DELAY: u32 = 500; // ms
const MAX_DELAYED_ACKS: u32 = 5;
const RESPONSE_TIMEOUT: u32 = 3000; // ms
const TIME_WAIT_TIMEOUT: u32 = 5000; // ms
const DEFAULT_TCP_MSS: usize = 536;

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
    Listen,
    SynReceived,
}

const FLAG_FIN: u8 = 1;
const FLAG_SYN: u8 = 2;
const FLAG_RST: u8 = 4;
const FLAG_PSH: u8 = 8;
const FLAG_ACK: u8 = 16;

pub type SocketReference = Arc<TCPSocket>;

// The condition in the socket reference is used to signal to clients of the
// API that are waiting, for example, for a reader waiting for new data or
// on open.
pub struct TCPSocket (Mutex<TCPSocketState>, Condvar);

struct TCPSocketState {
    remote_ip: util::IPAddr,
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
    // Variable names from RFC 9293, 3.3.1
    //
    // ----------|----------|----------|----------
    //        SND.UNA    SND.NXT    SND.UNA
    //                             +SND.WND
    //
    send_unacked: u32,           // SND.UNA
    send_next_seq: u32,          // SND.NXT
    send_window: u32,            // SND.WND
    send_last_win_seq: u32,      // SND.WL1
    send_last_win_ack: u32,      // SND.WL2


    retransmit_queue: buf::NetBuffer,
    retransmit_timer_id: i32,
    response_timer_id: i32,
    request_retry_count: u32,
    transmit_mss: usize,

    // Listen
    socket_queue: Vec<SocketReference>,
}

struct TCPReassembler {
    next_sequence: u32,
    out_of_order: Vec<(u32, buf::NetBuffer)>,
}

struct TCPSendParams<'a> {
    source_port: u16,
    dest_ip: util::IPAddr,
    dest_port: u16,
    seq_num: u32,
    ack_num: u32,
    flags: u8,
    window: u16,
    options: &'a [u8],
}

impl TCPSocket {
    fn new(remote_ip: util::IPAddr, remote_port: u16, local_port: u16) -> TCPSocket {
        TCPSocket(
            Mutex::new(TCPSocketState::new(remote_ip, remote_port, local_port)),
            Condvar::new(),
        )
    }

    fn lock(&self) -> (MutexGuard<TCPSocketState>, &Condvar) {
        (self.0.lock().unwrap(), &self.1)
    }
}

/// Each socket is uniquely identified by the tuple of remote_ip/remote_port/local_port
type SocketKey = (util::IPAddr, u16, u16);
type PortMap = HashMap<SocketKey, SocketReference>;

static PORT_MAP: LazyLock<Mutex<PortMap>> = LazyLock::new( || {
    Mutex::new(HashMap::new())
});

/// Generate a random ephemeral port that doesn't conflict with any open sockets.
fn find_ephemeral_port(
    guard: &mut MutexGuard<PortMap>,
    remote_ip: util::IPAddr,
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
    remote_ip: util::IPAddr,
    remote_port: u16,
) -> Result<SocketReference, &'static str> {
    let mut portmap_guard = PORT_MAP.lock().unwrap();
    let local_port = find_ephemeral_port(&mut portmap_guard, remote_ip, remote_port);
    let socket_ref = Arc::new(TCPSocket::new(remote_ip, remote_port, local_port));

    portmap_guard.insert((remote_ip, remote_port, local_port), socket_ref.clone());
    drop(portmap_guard);

    let (mut guard, cond) = (*socket_ref).lock();
    guard.set_state(TCPState::SynSent);

    guard.send_packet(buf::NetBuffer::new(), FLAG_SYN);
    set_response_timer(&mut guard, socket_ref.clone());

    // Wait until this is connected
    while !matches!(guard.state, TCPState::Established) {
        guard = cond.wait(guard).unwrap();
        if matches!(guard.state, TCPState::Closed) {
            return Err("Connection failed");
        }
    }

    std::mem::drop(guard);

    Ok(socket_ref)
}

pub fn tcp_close(socket_ref: &mut SocketReference) {
    let (mut guard, _) = (*socket_ref).lock();

    println!("{} tcp_close: state {:?}", guard, guard.state);
    match guard.state {
        TCPState::Listen => {
            let local_port = guard.local_port;
            guard.set_state(TCPState::Closed);
            drop(guard); // Unlock to avoid deadlock
            PORT_MAP
                .lock()
                .unwrap()
                .remove(&(util::IPAddr::new(), 0, local_port));
        }

        TCPState::Established => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_FIN | FLAG_ACK);
            set_response_timer(&mut guard, socket_ref.clone());
            guard.set_state(TCPState::FinWait1);
        }

        TCPState::CloseWait => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_FIN | FLAG_ACK);
            set_response_timer(&mut guard, socket_ref.clone());
            guard.set_state(TCPState::LastAck);
        }

        _ => {}
    }
}

pub fn tcp_read(socket_ref: &mut SocketReference, data: &mut [u8]) -> i32 {
    let (mut guard, cond) = (*socket_ref).lock();

    loop {
        if !matches!(guard.state, TCPState::Established) && guard.receive_queue.is_empty() {
            return -1;
        }

        if !guard.receive_queue.is_empty() {
            let got = guard.receive_queue.copy_to_slice(data);
            guard.receive_queue.trim_head(got);
            return got as i32;
        }

        guard = cond.wait(guard).unwrap();
    }
}

pub fn tcp_write(socket_ref: &mut SocketReference, data: &[u8]) -> i32 {
    let (mut guard, cond) = (*socket_ref).lock();

    if matches!(guard.state, TCPState::Closed) {
        return -1;
    }

    let mut offset = 0;
    while offset < data.len() {
        let packet_length = std::cmp::min(data.len() - offset, guard.transmit_mss);
        let max_segment = guard.send_unacked.wrapping_add(guard.send_window);
        if util::seq_gt(
            guard.send_next_seq.wrapping_add(packet_length as u32),
            max_segment,
        ) {
            // We are out of transmit window. Wait for acks to come in.
            println!(
                "{}: Waiting for transmit window to open, next_seq {} window_max {}",
                guard, guard.send_next_seq, max_segment
            );
            guard = cond.wait(guard).unwrap();
            println!("{}: Transmit window opened", guard);
            if matches!(guard.state, TCPState::Closed) {
                return offset as i32;
            }

            continue;
        }

        let mut packet = buf::NetBuffer::new();
        let packet_slice = &data[offset..offset + packet_length];
        packet.append_from_slice(packet_slice);
        guard.send_packet(packet, FLAG_ACK | FLAG_PSH);
        guard.send_next_seq = guard.send_next_seq.wrapping_add(packet_length as u32);
        guard.retransmit_queue.append_from_slice(packet_slice);
        offset += packet_length;

        if guard.retransmit_timer_id == -1 {
            let socket_arc = socket_ref.clone();
            guard.retransmit_timer_id = timer::set_timer(RETRANSMIT_INTERVAL, move || {
                retransmit(socket_arc);
            });
        }
    }

    assert!(offset == data.len());

    data.len() as i32
}

pub fn tcp_listen(port: u16) -> Result<SocketReference, &'static str> {
    let socket_ref = Arc::new(TCPSocket::new(util::IPAddr::new(), 0, port));

    let (mut guard, _cond) = (*socket_ref).lock();
    guard.set_state(TCPState::Listen);
    drop(guard);

    let mut portmap_guard = PORT_MAP.lock().unwrap();
    if portmap_guard.contains_key(&(util::IPAddr::new(), 0, port)) {
        return Err("Port already in use");
    }

    portmap_guard.insert((util::IPAddr::new(), 0, port), socket_ref.clone());
    drop(portmap_guard);

    Ok(socket_ref)
}

pub fn tcp_accept(socket_ref: &mut SocketReference) -> Result<SocketReference, &'static str>{
    let (mut guard, cond) = (*socket_ref).lock();

    while guard.socket_queue.is_empty() {
        guard = cond.wait(guard).unwrap();
    }

    Ok(guard.socket_queue.remove(0))
}

fn retransmit(socket_ref: SocketReference) {
    let (mut guard, _cond) = (*socket_ref).lock();

    if matches!(guard.state, TCPState::Closed) {
        return;
    }

    util::STATS.packets_retransmitted.inc();

    if !guard.retransmit_queue.is_empty() {
        println!("Retransmitting sequence {}", guard.send_next_seq);
        let mut packet = buf::NetBuffer::new();
        packet.append_from_buffer(&guard.retransmit_queue, guard.transmit_mss);
        guard.send_packet(packet, FLAG_ACK | FLAG_PSH);
        let socket_clone = socket_ref.clone();
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

impl TCPSocketState {
    fn new(remote_ip: util::IPAddr, remote_port: u16, local_port: u16) -> TCPSocketState {
        let initial_sequence = rand::random::<u32>();
        TCPSocketState {
            remote_ip,
            remote_port,
            local_port,
            send_next_seq: initial_sequence,
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
            transmit_mss: DEFAULT_TCP_MSS,
            socket_queue: Vec::new(),
            send_unacked: initial_sequence,
            send_window: 0,
            send_last_win_seq: 0,
            send_last_win_ack: 0,
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
            "{}: send_packet: flags {} seq {} ack {} window {} (length {})",
            self,
            flags_to_str(flags),
            self.send_next_seq,
            ack_seq,
            receive_window,
            packet.len(),
        );

        let options = if (flags & FLAG_SYN) != 0 {
            &[2, 4, 0x5, 0xdc].as_slice() // MSS 1500
        } else {
            &[].as_slice()
        };

        let params = TCPSendParams {
            source_port: self.local_port,
            dest_ip: self.remote_ip,
            dest_port: self.remote_port,
            seq_num: self.send_next_seq,
            ack_num: ack_seq,
            flags,
            window: receive_window,
            options,
        };

        tcp_output(packet, &params);
    }

    fn set_state(&mut self, new_state: TCPState) {
        println!(
            "{}: Change state from {:?} to {:?}",
            self, self.state, new_state
        );
        self.state = new_state;
        self.request_retry_count = 0;
    }

    fn is_established(&self) -> bool  {
        !matches!(
            self.state,
            TCPState::Closed | TCPState::SynSent | TCPState::TimeWait
        )
    }
}

impl Display for TCPSocketState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "localhost:{} {}:{}",
            self.local_port, self.remote_ip, self.remote_port
        )
    }
}

impl TCPReassembler {
    const fn new() -> TCPReassembler {
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

pub fn tcp_input(mut packet: buf::NetBuffer, source_ip: util::IPAddr) {
    if !validate_checksum(&packet, source_ip) {
        println!("TCP checksum error");
        return;
    }

    // Decode header
    let packet_length = packet.len();
    let header = packet.header_mut();
    let source_port = util::get_be16(&header[0..2]);
    let dest_port = util::get_be16(&header[2..4]);
    let seq_num = util::get_be32(&header[4..8]);
    let ack_num = util::get_be32(&header[8..12]);
    let header_length = ((header[12] >> 4) * 4) as usize;
    let remote_window_size = util::get_be16(&header[14..16]);
    let flags = header[13];

    println!(
        "tcp_input: source_ip {} source_port {} dest_port {} flags {} seq {} ack {} window {} ({} bytes of data)",
        source_ip,
        source_port,
        dest_port,
        flags_to_str(flags),
        seq_num,
        ack_num,
        remote_window_size,
        packet_length - header_length,
    );

    // Parse options
    let options = parse_options(&header[20..header_length]);
    packet.trim_head(header_length);

    // Lookup socket
    let mut port_map_guard = PORT_MAP.lock().unwrap();
    let pm_entry = port_map_guard.get_mut(&(source_ip, source_port, dest_port));
    if pm_entry.is_none() {
        // This might be a new socket, check for a listen socket
        let listen_entry = port_map_guard.get_mut(&(util::IPAddr::new(), 0, dest_port));
        if listen_entry.is_none() || (flags & FLAG_SYN) == 0 {
            let response = buf::NetBuffer::new();
            let params = TCPSendParams {
                source_port: dest_port,
                dest_ip: source_ip,
                dest_port: source_port,
                seq_num: 1,
                ack_num: seq_num + 1,
                flags: FLAG_RST | FLAG_ACK,
                window: 0,
                options: &[],
            };

            tcp_output(response, &params);
            return;
        }

        let listen_socket = listen_entry
            .expect("just checked if listen_entry is none above")
            .clone();
        let new_socket = handle_new_connection(
            listen_socket,
            source_ip,
            source_port,
            dest_port,
            seq_num,
            ack_num,
            remote_window_size,
            options.max_segment_size
        );

        port_map_guard.insert((source_ip, source_port, dest_port), new_socket);
        return;
    }

    let socket_ref = pm_entry
        .expect("just checked if pm_entry is none above")
        .clone();
    let (mut guard, cond) = (*socket_ref).lock();

    if options.max_segment_size != 0 {
        guard.transmit_mss = options.max_segment_size;
        println!("Set max segment size {}", options.max_segment_size);
    }

    // XXX hack: this should be reset inside the state transitions for
    // each corresponding path.
    if guard.response_timer_id != -1 {
        timer::cancel_timer(guard.response_timer_id);
        guard.response_timer_id = -1;
    }

    // XXX hack. Handling of this differs.
    if (flags & FLAG_RST) != 0 {
        println!("{}: Connection reset", guard);
        guard.set_state(TCPState::Closed);
        cond.notify_all();
        return;
    }

    if !packet.is_empty() {
        // Handle received data
        guard.highest_seq_received = util::wrapping_max(
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
                let socket_clone = socket_ref.clone();
                guard.delayed_ack_timer_id = timer::set_timer(MAX_ACK_DELAY, move || {
                    let (mut guard, _cond) = (*socket_clone).lock();
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

    if (flags & FLAG_ACK) != 0 && guard.is_established() {
        // RFC 9293, 3.10.7.4 [SEGMENT ARRIVES] Other States
        // Fifth, check the ACK field
       if util::seq_lt(guard.send_unacked, ack_num)
            && util::seq_le(ack_num, guard.send_next_seq)
        {
            let trim = ack_num.wrapping_sub(guard.send_unacked) as usize;
            println!("{}: trim {} retransmit_queue size {}", guard, trim, guard.retransmit_queue.len());
            guard.retransmit_queue.trim_head(trim);
            println!(
                "{}: Trimming {} acked bytes from retransmit queue, size is now {}",
                guard,
                trim,
                guard.retransmit_queue.len()
            );

            if guard.retransmit_queue.is_empty() {
                timer::cancel_timer(guard.retransmit_timer_id);
                guard.retransmit_timer_id = -1;
            }

            guard.send_unacked = ack_num;
        }

        // We record the acknowledgement and sequence number of
        // the last window update in the send_last_win_seq and
        // send_last_win_ack fields to prevent using old segments
        // to update the window.
        if util::seq_le(guard.send_unacked, ack_num)
            && util::seq_le(ack_num, guard.send_next_seq)
            && (util::seq_lt(guard.send_last_win_seq, seq_num)
            || (guard.send_last_win_seq == seq_num
                && util::seq_le(guard.send_last_win_ack, ack_num)))
        {
            guard.send_window = remote_window_size as u32;
            guard.send_last_win_seq = seq_num;
            guard.send_last_win_ack = ack_num;
            cond.notify_all();
        }
    }

    match guard.state {
        TCPState::SynSent => {
            if (flags & FLAG_ACK) != 0 {
                guard.set_state(TCPState::Established);
                guard.highest_seq_received = seq_num.wrapping_add(1);
                guard.reassembler.set_next_expect(seq_num.wrapping_add(1));

                guard.send_window = remote_window_size as u32;
                guard.send_last_win_seq = seq_num;
                guard.send_last_win_ack = ack_num;
                guard.send_unacked = ack_num;

                // The SYN consumes a sequence number.
                guard.send_next_seq = guard.send_next_seq.wrapping_add(1);

                // Send ack to complete handshake
                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                set_response_timer(&mut guard, socket_ref.clone());

                // Wake up thread waiting in connect
                cond.notify_all();
            }
        }

        TCPState::SynReceived => {
            if (flags & FLAG_ACK) != 0 {
                guard.set_state(TCPState::Established);

                // The SYN consumes a sequence number.
                guard.send_next_seq = guard.send_next_seq.wrapping_add(1);
                guard.send_unacked = ack_num;
            }
        }

        TCPState::Established => {
            if (flags & FLAG_FIN) != 0 {
                guard.set_state(TCPState::CloseWait);

                // Ack will be sent below. FIN packets can contain data.
                cond.notify_all();
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
                && ack_num == guard.send_next_seq.wrapping_add(1)
            {
                guard.set_state(TCPState::TimeWait);
                let socket_clone = socket_ref.clone();
                timer::set_timer(TIME_WAIT_TIMEOUT, move || {
                    time_wait_timeout(socket_clone);
                });
            } else if (flags & FLAG_FIN) != 0 {
                guard.set_state(TCPState::Closing);
                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                set_response_timer(&mut guard, socket_ref.clone());
            } else if (flags & FLAG_ACK) != 0 && ack_num == guard.send_next_seq.wrapping_add(1)
            {
                guard.set_state(TCPState::FinWait2);
            }
        }

        TCPState::FinWait2 => {
            if (flags & FLAG_FIN) != 0 {
                guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
                set_response_timer(&mut guard, socket_ref.clone());
                guard.set_state(TCPState::TimeWait);
                let socket_clone = socket_ref.clone();
                timer::set_timer(TIME_WAIT_TIMEOUT, move || {
                    time_wait_timeout(socket_clone);
                });
            }
        }

        TCPState::Closing => {
            if (flags & FLAG_ACK) != 0 {
                guard.set_state(TCPState::TimeWait);
                let socket_clone = socket_ref.clone();
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

fn validate_checksum(packet: &buf::NetBuffer, source_ip: util::IPAddr) -> bool {
    let dest_ip = if matches!(source_ip, util::IPAddr::V4(_)) {
        netif::get_ipaddr().0
    } else {
        netif::get_ipaddr().1
    };

    let ph_checksum = util::compute_pseudo_header_checksum(
        source_ip,
        dest_ip,
        packet.len(),
        ip::PROTO_TCP,
    );

    let checksum = util::compute_buffer_ones_comp(ph_checksum, packet) ^ 0xffff;
    checksum == 0
}

struct TCPHeaderOptions {
    max_segment_size: usize,
}

fn parse_options(header: &[u8]) -> TCPHeaderOptions {
    let mut options = TCPHeaderOptions {
        max_segment_size: 0,
    };

    let mut opt_offset = 0;
    while opt_offset < header.len() {
        let option_type = header[opt_offset];
        if option_type == 0 {
            break;
        }

        if option_type == 1 {
            // No-op
            opt_offset += 1;
            continue;
        }

        let option_length = header[opt_offset + 1] as usize;
        if option_type == 0 {
            break;
        }

        if option_type == 2 {
            options.max_segment_size = util::get_be16(&header[opt_offset + 2..opt_offset + 4]) as usize;
        }

        println!("offset {} option {} length {}", opt_offset, option_type, option_length);
        opt_offset += option_length;
    }

    options
}

fn handle_new_connection(
    listen_socket_ref: SocketReference,
    source_ip: util::IPAddr,
    source_port: u16,
    dest_port: u16,
    seq_num: u32,
    ack_num: u32,
    remote_window_size: u16,
    max_segment_size: usize,
) -> SocketReference {
    println!(
        "New connection from {}:{} to {}",
        source_ip, source_port, dest_port
    );
    let new_socket_ref = Arc::new(TCPSocket::new(source_ip, source_port, dest_port));

    let (mut guard, _cond) = (*new_socket_ref).lock();
    guard.remote_ip = source_ip;
    guard.remote_port = source_port;
    guard.set_state(TCPState::SynReceived);
    guard.transmit_mss = max_segment_size;
    guard.highest_seq_received = seq_num.wrapping_add(1);
    guard.reassembler.set_next_expect(seq_num.wrapping_add(1));

    guard.send_packet(buf::NetBuffer::new(), FLAG_SYN | FLAG_ACK);
    guard.send_unacked = seq_num;
    guard.send_last_win_ack = ack_num;
    guard.send_last_win_seq = seq_num;
    guard.send_window = remote_window_size as u32;
    set_response_timer(&mut guard, new_socket_ref.clone());
    drop(guard); // Unlock to avoid deadlock

    let (mut guard, cond) = (*listen_socket_ref).lock();
    assert!(
        matches!(guard.state, TCPState::Listen),
        "Listen socket should be in listen state",
    );

    guard.socket_queue.push(new_socket_ref.clone());
    cond.notify_all();

    new_socket_ref
}

fn tcp_output(mut packet: buf::NetBuffer, params: &TCPSendParams) {
    assert!(params.options.len() % 4 == 0); // Must be pre-padded
    let header_length = TCP_HEADER_LEN + params.options.len();
    packet.alloc_header(header_length);
    let packet_length = packet.len() as u16;
    {
        let header = packet.header_mut();
        util::set_be16(&mut header[0..2], params.source_port);
        util::set_be16(&mut header[2..4], params.dest_port);
        util::set_be32(&mut header[4..8], params.seq_num);
        util::set_be32(&mut header[8..12], params.ack_num);
        header[12] = ((header_length / 4) << 4) as u8; // Data offset
        header[13] = params.flags;
        util::set_be16(&mut header[14..16], params.window);
        if !params.options.is_empty() {
            header[20..20 + params.options.len()].copy_from_slice(params.options);
        }
    }

    // Compute checksum
    // First need to create a pseudo header
    let ph_checksum = util::compute_pseudo_header_checksum(
        if matches!(params.dest_ip, util::IPAddr::V4(_)) {
            netif::get_ipaddr().0
        } else {
            netif::get_ipaddr().1
        },
        params.dest_ip,
        packet_length as usize,
        ip::PROTO_TCP,
    );

    let checksum = util::compute_buffer_ones_comp(ph_checksum, &packet) ^ 0xffff;

    let header = packet.header_mut();
    util::set_be16(&mut header[16..18], checksum);

    ip::ip_output(packet, ip::PROTO_TCP, params.dest_ip);
}

fn set_response_timer(guard: &mut MutexGuard<TCPSocketState>, socket_ref: SocketReference) {
    if guard.response_timer_id != -1 {
        timer::cancel_timer(guard.response_timer_id);
    }

    let socket_clone = socket_ref.clone();
    guard.response_timer_id = timer::set_timer(RESPONSE_TIMEOUT, move || {
        response_timeout(socket_clone);
    });
}

fn response_timeout(socket_ref: SocketReference) {
    let (mut guard, cond) = (*socket_ref).lock();

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
            set_response_timer(&mut guard, socket_ref.clone());
        }

        TCPState::FinWait1 | TCPState::LastAck => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_FIN);
            set_response_timer(&mut guard, socket_ref.clone());
        }

        TCPState::Closing | TCPState::CloseWait => {
            guard.send_packet(buf::NetBuffer::new(), FLAG_ACK);
            set_response_timer(&mut guard, socket_ref.clone());
        }

        _ => {
            // This would indicate a bug: we set a timer where we shoudn't have.
            panic!(
                "{}: unexpected state in response_timeout: {:?}",
                guard, guard.state
            );
        }
    }

    set_response_timer(&mut guard, socket_ref.clone());
}

fn time_wait_timeout(socket_ref: SocketReference) {
    let (mut guard, _cond) = (*socket_ref).lock();

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
