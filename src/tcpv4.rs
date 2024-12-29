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
use crate::util;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Condvar;
use std::sync::{Arc, Mutex};


/// Each socket is uniquely identified by the tuple of remote_ip/remote_port/local_port
type SocketKey = (util::IPv4Addr, u16, u16);
const EPHEMERAL_PORT_BASE: u16 = 49152;
const TCP_MTU: usize = 1500;

const MAX_RECEIVE_WINDOW: u16 = 0xffff;

enum TCPState {
    Closed,
    SynSent,
    Established,
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

    // Transmit
    next_transmit_seq: u32,
    retransmit_queue: buf::NetBuffer,
    transmit_window_max: u32, // Highest sequence we can transmit
}

pub struct TCPReassembler {
    next_sequence: u32,
    out_of_order: Vec<(u32, buf::NetBuffer)>,
}

lazy_static! {
    static ref PORT_MAP: Mutex<HashMap<SocketKey, Arc<Mutex<TCPSocket>>>> = Mutex::new(HashMap::new());

    // This is not ideal, as it wakes up all threads waiting for data any time there
    // is actitiy on any socket. But we get into all kinds of reference/ownership
    // complexity if we try to associate a condition with each socket.
    static ref RECV_WAIT: Condvar = Condvar::new();
}

// XXX this is not protected with a lock.
static mut NEXT_EPHEMERAL_PORT: u16 = EPHEMERAL_PORT_BASE;

impl TCPSocket {
    fn new(remote_ip: util::IPv4Addr, remote_port: u16, local_port: u16) -> TCPSocket {
        TCPSocket {
            remote_ip: remote_ip,
            remote_port: remote_port,
            local_port: local_port,
            next_transmit_seq: 1, // XXX this should be randomized
            transmit_window_max: 0,
            state: TCPState::Closed,
            receive_queue: buf::NetBuffer::new(),
            reassembler: TCPReassembler::new(),
            retransmit_queue: buf::NetBuffer::new(),
        }
    }

    fn get_receive_win_size(&self) -> u16 {
        MAX_RECEIVE_WINDOW - self.receive_queue.len() as u16
    }

    fn handle_packet(
        &mut self,
        packet: buf::NetBuffer,
        seq_num: u32,
        ack_num: u32,
        window_size: u16,
        flags: u8
    ) {
        match self.state {
            TCPState::SynSent => {
                if (flags & FLAG_RST) != 0 {
                    println!("Connection refused");
                    self.state = TCPState::Closed;
                    RECV_WAIT.notify_all();
                    return;
                }

                if (flags & FLAG_ACK) != 0 {
                    if ack_num != self.next_transmit_seq.wrapping_add(1) {
                        println!("Unexpected ack {} wanted {}+1", ack_num, self.next_transmit_seq);
                    }

                    self.state = TCPState::Established;
                    self.reassembler.set_next_expect(seq_num + 1);

                    // The SYN consumes a sequence number.
                    self.next_transmit_seq = self.next_transmit_seq.wrapping_add(1);

                    tcp_output(
                        buf::NetBuffer::new(),
                        self.local_port,
                        self.remote_ip,
                        self.remote_port,
                        self.next_transmit_seq,
                        self.reassembler.get_next_expect(),
                        FLAG_ACK,
                        self.get_receive_win_size(),
                    );

                    // Wake up thread waiting in connect
                    RECV_WAIT.notify_all();
                }
            }

            TCPState::Established => {
                if (flags & FLAG_FIN) != 0 {
                    // XXX hack: should actually go into a closing state.
                    self.state = TCPState::Closed;
                    return;
                }

                if (flags & FLAG_RST) != 0 {
                    println!("Connection reset");
                    self.state = TCPState::Closed;
                    return;
                }

                if (flags & FLAG_ACK) != 0 {
                    if util::seq_gt(ack_num, self.next_transmit_seq) {
                        println!("ERROR: Unexpected ack {} next_transmit_seq {}",
                            ack_num, self.next_transmit_seq);
                    } else {
                        let oldest_unacked = self.next_transmit_seq.wrapping_sub(
                            self.retransmit_queue.len() as u32);
                        if util::seq_gt(ack_num, oldest_unacked) {
                            let trim = ack_num.wrapping_sub(oldest_unacked) as usize;
                            self.retransmit_queue.trim_head(trim);
                            println!(
                                "Trimming {} acked bytes from retransmit queue, size is now {}",
                                trim, self.retransmit_queue.len()
                            );
                        }
                    }

                    self.transmit_window_max = ack_num.wrapping_add(window_size as u32);
                }

                let got = self.reassembler.add_packet(packet, seq_num);
                if got.is_some() {
                    self.receive_queue.append_buffer(got.unwrap());
                }

                // Acknowledge packet
                // TODO: this should use a timer to delay the ack so it's not spammy.
                tcp_output(
                    buf::NetBuffer::new(),
                    self.local_port,
                    self.remote_ip,
                    self.remote_port,
                    self.next_transmit_seq,
                    self.reassembler.get_next_expect(),
                    FLAG_ACK,
                    self.get_receive_win_size(),
                );

                RECV_WAIT.notify_all();
            }
            _ => {
                println!("Unhandled state");
            }
        }
    }
}

pub fn tcp_open(remote_ip: util::IPv4Addr, remote_port: u16)
    -> Result<Arc<Mutex<TCPSocket>>, &'static str>
{
    let local_port = unsafe { NEXT_EPHEMERAL_PORT };
    unsafe {
        NEXT_EPHEMERAL_PORT += 1;
    }

    let handle = Arc::new(Mutex::new(TCPSocket::new(
        remote_ip,
        remote_port,
        local_port,
    )));

    PORT_MAP
        .lock()
        .unwrap()
        .insert((remote_ip, remote_port, local_port), handle.clone());

    let mut guard = handle.lock().unwrap();
    guard.state = TCPState::SynSent;

    // TODO: Should set a timer to retry this if it isn't acknowledged.
    tcp_output(
        buf::NetBuffer::new(),
        guard.local_port,
        remote_ip,
        remote_port,
        guard.next_transmit_seq,
        0,
        FLAG_SYN,
        guard.get_receive_win_size(),
    );

    // Wait until this is connected
    while !matches!(guard.state ,TCPState::Established) {
        guard = RECV_WAIT.wait(guard).unwrap();
        if matches!(guard.state, TCPState::Closed) {
            return Err("Connection refused");
        }
    }

    std::mem::drop(guard);

    Ok(handle)
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

pub fn tcp_read(socket: &mut Arc<Mutex<TCPSocket>>, data: &mut [u8]) -> i32 {
    let mut guard = socket.lock().unwrap();
    loop {
        if matches!(guard.state, TCPState::Closed) {
            return -1;
        }

        if guard.receive_queue.len() > 0 {
            let got = guard.receive_queue.copy_to_slice(data);
            guard.receive_queue.trim_head(got);
            return got as i32;
        }

        guard = RECV_WAIT.wait(guard).unwrap();
    }
}

pub fn tcp_write(socket: &mut Arc<Mutex<TCPSocket>>, data: &[u8]) -> i32 {
    assert!(data.len() < TCP_MTU); // XXX Fix this at some point

    let mut guard = socket.lock().unwrap();

    if matches!(guard.state, TCPState::Closed) {
        return -1;
    }

    if util::seq_gt(guard.next_transmit_seq.wrapping_add(data.len() as u32),
        guard.transmit_window_max) {
        // XXX Window is full, can't write. Need to block.
        return 0;
    }

    let mut packet = buf::NetBuffer::new();
    packet.append_from_slice(data);
    tcp_output(
        packet,
        guard.local_port,
        guard.remote_ip,
        guard.remote_port,
        guard.next_transmit_seq,
        guard.reassembler.get_next_expect(),
        FLAG_ACK | FLAG_PSH,
        guard.get_receive_win_size(),
    );

    // XXX Set the retransmit timer if it is not already pending.

    guard.next_transmit_seq = guard.next_transmit_seq.wrapping_add(data.len() as u32);
    guard.retransmit_queue.append_from_slice(data);
    data.len() as i32
}

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
    let win_size = util::get_be16(&header[14..16]);
    let flags = header[13];

    packet.trim_head(header_size);

    // Lookup socket
    let mut port_map_guard = PORT_MAP.lock().unwrap();
    let socket = port_map_guard.get_mut(&(source_ip, source_port, dest_port));
    if socket.is_none() {
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

    socket
        .unwrap()
        .lock()
        .unwrap()
        .handle_packet(packet, seq_num, ack_num, win_size, flags);
}

const TCP_HEADER_LEN: usize = 20;

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
    util::set_be32(&mut pseudo_header[0..4], netif::get_ipaddr());
    util::set_be32(&mut pseudo_header[4..8], dest_ip);
    pseudo_header[8] = 0; // Reserved
    pseudo_header[9] = ipv4::PROTO_TCP; // Protocol
    util::set_be16(&mut pseudo_header[10..12], length); // TCP length (header + data)

    let ph_sum = util::compute_ones_comp(0, &pseudo_header);
    let checksum = util::compute_buffer_ones_comp(ph_sum, &packet) ^ 0xffff;

    let header = packet.header_mut();
    util::set_be16(&mut header[16..18], checksum);

    ipv4::ip_output(packet, ipv4::PROTO_TCP, dest_ip);
}

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

