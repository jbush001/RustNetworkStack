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

enum TCPState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
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
    next_seq_num: u32,
    next_expected_seq: u32,
    state: TCPState,
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
            next_expected_seq: 0,
            state: TCPState::Closed,
        }
    }

    fn handle_packet(&mut self, packet: buf::NetBuffer, seq_num: u32, ack_num: u32, flags: u8) {
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
                    self.next_expected_seq = seq_num + 1;

                    println!("Connection established");
                    tcp_output(
                        buf::NetBuffer::new(),
                        self.local_port,
                        self.remote_ip,
                        self.remote_port,
                        self.next_seq_num,
                        self.next_expected_seq,
                        FLAG_ACK,
                        32768,
                    );
                }
            }
            _ => {
                println!("Unhandled state");
            }
        }
    }
}

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
        let mut sock = handle.lock().unwrap();
        sock.state = TCPState::SynSent;

        // XXX will not retry
        tcp_output(
            buf::NetBuffer::new(),
            local_port,
            remote_ip,
            remote_port,
            sock.next_seq_num,
            0,
            FLAG_SYN,
            32768,
        );

        sock.next_seq_num += 1;
    }

    handle
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

pub fn tcp_input(packet: buf::NetBuffer, source_ip: util::IPv4Addr) {
    let payload = packet.payload();
    let source_port = util::get_be16(&payload[0..2]);
    let dest_port = util::get_be16(&payload[2..4]);
    let seq_num = util::get_be32(&payload[4..8]);
    let ack_num = util::get_be32(&payload[8..12]);
    let window = util::get_be16(&payload[14..16]);
    let flags = payload[13];

    println!("source port {} dest port {}", source_port, dest_port);
    println!("sequence {} ack {}", seq_num, ack_num);
    println!("window {}", window);
    println!(
        "Flags {}{}{}{}{}",
        if (flags & FLAG_ACK) != 0 { "A" } else { "-" },
        if (flags & FLAG_PSH) != 0 { "P" } else { "-" },
        if (flags & FLAG_RST) != 0 { "R" } else { "-" },
        if (flags & FLAG_SYN) != 0 { "S" } else { "-" },
        if (flags & FLAG_FIN) != 0 { "F" } else { "-" }
    );

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
    packet.add_header(TCP_HEADER_LEN);
    let payload = packet.mut_payload();
    let length = payload.len() as u16;

    util::set_be16(&mut payload[0..2], source_port);
    util::set_be16(&mut payload[2..4], dest_port);
    util::set_be32(&mut payload[4..8], seq_num);
    util::set_be32(&mut payload[8..12], ack_num);
    payload[12] = ((TCP_HEADER_LEN / 4) << 4) as u8; // Data offset
    payload[13] = flags;
    util::set_be16(&mut payload[14..16], window);

    // Compute checksum
    // First need to create a pseudo header
    let mut pseudo_header = [0u8; 12];
    util::set_be32(&mut pseudo_header[0..4], netif::get_ipaddr());
    util::set_be32(&mut pseudo_header[4..8], dest_ip);
    pseudo_header[8] = 0; // Reserved
    pseudo_header[9] = ipv4::PROTO_TCP; // Protocol
    util::set_be16(&mut pseudo_header[10..12], length); // TCP length (header + data)

    let ph_sum = util::compute_ones_complement(0, &pseudo_header);
    let checksum = util::compute_ones_complement(ph_sum, &payload[..length as usize]) ^ 0xffff;
    util::set_be16(&mut payload[16..18], checksum);

    ipv4::ip_output(packet, ipv4::PROTO_TCP, dest_ip);
}
