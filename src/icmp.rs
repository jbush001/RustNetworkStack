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

use crate::packet;
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


pub fn icmp_recv(pkt: packet::NetworkPacket, source_ip: u32) {
    let payload = &pkt.data[pkt.offset as usize..pkt.length as usize];
    let checksum = util::compute_checksum(&payload);
    if checksum != 0 {
        print!("ICMP checksum error");
        return;
    }

    let packet_type = payload[0];
    if packet_type == ICMP_ECHO_REQUEST {
        // Send a response
        let body = &pkt.data[(pkt.offset + ICMP_HEADER_LEN) as usize..pkt.length as usize];

        let mut new_packet = packet::NetworkPacket {
            data: [0; 2048],
            length: (64 + body.len()) as u32,
            offset: 64
        };

        for i in 0..body.len() {
            new_packet.data[new_packet.offset as usize + i] = body[i];
        }

        icmp_send(new_packet, ICMP_ECHO_REPLY, source_ip);
    }
}

pub fn icmp_send(mut pkt: packet::NetworkPacket, packet_type: u8, dest_addr: u32) {
    assert!(pkt.offset > ICMP_HEADER_LEN);
    pkt.offset -= ICMP_HEADER_LEN;
    let payload = &mut pkt.data[pkt.offset as usize..pkt.length as usize];
    payload[0] = packet_type;
    let checksum = util::compute_checksum(payload);
    util::set_be16(&mut payload[2..4], checksum);
    ip::ip_send(pkt, ip::PROTO_ICMP, dest_addr);
}

