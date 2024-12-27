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

const FLAG_FIN: u8 = 1;
const FLAG_SYN: u8 = 2;
const FLAG_RST: u8 = 4;
const FLAG_PSH: u8 = 8;
const FLAG_ACK: u8 = 16;

pub fn tcp_input(packet: buf::NetBuffer, source_ip: util::IPv4Addr) {
    println!("Got TCP packet");

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

    // We always send a reset packet, since we don't have any socket support
    // yet.
    let response = buf::NetBuffer::new();
    tcp_output(
        response,
        source_ip,
        dest_port,
        source_port,
        1, // Sequence number
        seq_num + 1, // Acknowledge sequence from host.
        FLAG_RST | FLAG_ACK,
        0,
    );
}

const TCP_HEADER_LEN: usize = 20;

pub fn tcp_output(
    mut packet: buf::NetBuffer,
    dest_ip: util::IPv4Addr,
    source_port: u16,
    dest_port: u16,
    seq_num: u32,
    ack_num: u32,
    flags: u8,
    window: u16,
) {
    println!("Sending TCP packet, ack {}", ack_num);
    packet.add_header(TCP_HEADER_LEN);
    let length = packet.payload_len() as u16;
    let payload = packet.mut_payload();

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

    util::print_binary(&packet.payload());

    ipv4::ip_output(packet, ipv4::PROTO_TCP, dest_ip);
}
