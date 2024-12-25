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
use crate::util;
use crate::ip;

//    0               1               2               3
//    +---------------+---------------+-----+-------------------------+
//  0 |     Type      |     Code      |          Checksum             |
//    +---------------+---------------+-------------------------------+
//  4 |                     Payload...                                |
//    +---------------------------------------------------------------+


const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;

const ICMP_HEADER_LEN: u32 = 4;


pub fn icmp_recv(packet: buf::NetBuffer, source_ip: util::IPv4Addr) {
    let payload = &packet.data[packet.offset as usize..packet.length as usize];
    let checksum = util::compute_checksum(&payload);
    if checksum != 0 {
        print!("ICMP checksum error");
        return;
    }

    let packet_type = payload[0];
    if packet_type == ICMP_ECHO_REQUEST {
        // Send a response
        let body = &packet.data[(packet.offset + ICMP_HEADER_LEN) as usize..packet.length as usize];

        let mut new_packet = buf::NetBuffer {
            data: [0; 2048],
            length: (64 + body.len()) as u32,
            offset: 64
        };

        new_packet.data[new_packet.offset as usize..new_packet.offset as usize + body.len()].copy_from_slice(body);
        icmp_send(new_packet, ICMP_ECHO_REPLY, source_ip);
    }
}

pub fn icmp_send(mut packet: buf::NetBuffer, packet_type: u8, dest_addr: util::IPv4Addr) {
    assert!(packet.offset > ICMP_HEADER_LEN);
    packet.offset -= ICMP_HEADER_LEN;
    let payload = &mut packet.data[packet.offset as usize..packet.length as usize];
    payload[0] = packet_type;
    let checksum = util::compute_checksum(payload);
    util::set_be16(&mut payload[2..4], checksum);
    ip::ip_send(packet, ip::PROTO_ICMP, dest_addr);
}

