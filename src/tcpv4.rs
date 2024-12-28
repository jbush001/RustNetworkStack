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

type SocketKey = (util::IPv4Addr, u16, u16);
const EPHEMERAL_PORT_BASE: u16 = 49152;

// XXX hardcoded for now, as this should scale with capacity.
const WINDOW_SIZE: u16 = 32768;

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
    next_seq_num: u32,
    state: TCPState,
    receive_queue: buf::NetBuffer,
    reassembler: TCPReassembler,
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
            next_seq_num: 1, // XXX this should be randomized
            state: TCPState::Closed,
            receive_queue: buf::NetBuffer::new(),
            reassembler: TCPReassembler::new(),
        }
    }

    fn handle_packet(
        &mut self,
        packet: buf::NetBuffer,
        seq_num: u32,
        _ack_num: u32,
        flags: u8
    ) {
        match self.state {
            TCPState::SynSent => {
                if (flags & FLAG_RST) != 0 {
                    println!("Connection refused");
                    self.state = TCPState::Closed;
                    return;
                }

                if (flags & FLAG_ACK) != 0 {
                    // XXX check that the ack number is correct

                    self.state = TCPState::Established;
                    self.reassembler.set_next_expect(seq_num + 1);

                    tcp_output(
                        buf::NetBuffer::new(),
                        self.local_port,
                        self.remote_ip,
                        self.remote_port,
                        self.next_seq_num,
                        self.reassembler.get_next_expect(),
                        FLAG_ACK,
                        WINDOW_SIZE,
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

                let got = self.reassembler.add_packet(packet, seq_num);
                if got.is_some() {
                    self.receive_queue.append_buffer(got.unwrap());
                }

                // Acknowledge packet
                tcp_output(
                    buf::NetBuffer::new(),
                    self.local_port,
                    self.remote_ip,
                    self.remote_port,
                    self.next_seq_num,
                    self.reassembler.get_next_expect(),
                    FLAG_ACK,
                    WINDOW_SIZE,
                );

                RECV_WAIT.notify_all();
            }
            _ => {
                println!("Unhandled state");
            }
        }
    }
}

/// XXX should probably return Option
pub fn tcp_open(remote_ip: util::IPv4Addr, remote_port: u16) -> Arc<Mutex<TCPSocket>> {
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

    {
        let mut guard = handle.lock().unwrap();
        guard.state = TCPState::SynSent;

        // XXX will not retry
        tcp_output(
            buf::NetBuffer::new(),
            local_port,
            remote_ip,
            remote_port,
            guard.next_seq_num,
            0,
            FLAG_SYN,
            32768,
        );

        guard.next_seq_num += 1;

        // Wait until this is connected
        while !matches!(guard.state ,TCPState::Established) {
            guard = RECV_WAIT.wait(guard).unwrap();
            // XXX this doesn't handle connection errors.
        }
    }

    handle
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
            self.next_sequence += packet.len() as u32;

            // Check if any of the out-of-order packets can now be reassembled.
            let mut i = 0;
            while i < self.out_of_order.len() {
                // XXX todo: if this packet is before the current one, remove it.
                // This is a bit tricky because we need to do a wrapped compare.

                if self.out_of_order[i].0 == self.next_sequence {
                    let (_, ooo_packet) = self.out_of_order.remove(i);
                    self.next_sequence += ooo_packet.len() as u32;
                    packet.append_buffer(ooo_packet);
                    i = 0;
                } else {
                    i += 1;
                }
            }

            Some(packet)
        } else {
            self.out_of_order.push((seq_num, packet));
            None
        }
    }

    fn get_next_expect(&self) -> u32 {
        self.next_sequence
    }
}

pub fn tcp_recv(socket: &mut Arc<Mutex<TCPSocket>>, data: &mut [u8]) -> i32 {
    let mut guard = socket.lock().unwrap();
    loop {
        if matches!(guard.state, TCPState::Closed) {
            return -1;
        }

        if guard.receive_queue.len() > 0 {
            let got = guard.receive_queue.copy_to_slice(data, usize::MAX);
            guard.receive_queue.trim_head(got);
            return got as i32;
        }

        guard = RECV_WAIT.wait(guard).unwrap();
    }
}

// XXX this is a hack for now, as it doesn't handle retransmit or buffering.
pub fn tcp_send(socket: &mut Arc<Mutex<TCPSocket>>, data: &[u8]) {
    let mut guard = socket.lock().unwrap();
    let mut packet = buf::NetBuffer::new();
    assert!(data.len() < 1460); // There's an MTU in there somewhere.
    packet.append_from_slice(data);

    tcp_output(
        packet,
        guard.local_port,
        guard.remote_ip,
        guard.remote_port,
        guard.next_seq_num,
        guard.reassembler.get_next_expect(),
        FLAG_ACK | FLAG_PSH,
        WINDOW_SIZE,
    );

    guard.next_seq_num += data.len() as u32;
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
        .handle_packet(packet, seq_num, ack_num, flags);
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
    fn test_reassemble1() {
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
        let got = new_packet.copy_to_slice(&mut data, 5);
        assert_eq!(got, 5);
    }

    #[test]
    fn test_reassemble2() {
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
        new_packet.copy_to_slice(&mut data, 200);
        assert!(data[0] == 1);
        assert!(data[99] == 1);
        assert!(data[100] == 2);
        assert!(data[199] == 2);
    }
}