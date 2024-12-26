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
use crate::util;

//    0               1               2               3
//    +---------------+---------------+-----+-------------------------+
//  0 |     Type      |     Code      |          Checksum             |
//    +---------------+---------------+-------------------------------+
//  4 |                     Payload...                                |
//    +---------------------------------------------------------------+

const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;

const ICMP_HEADER_LEN: usize = 4;

pub fn icmp_recv(mut packet: buf::NetBuffer, source_ip: util::IPv4Addr) {
    let payload = packet.payload();
    let checksum = util::compute_checksum(&payload);
    if checksum != 0 {
        print!("ICMP checksum error");
        return;
    }

    let packet_type = payload[0];
    packet.remove_header(ICMP_HEADER_LEN);
    if packet_type == ICMP_ECHO_REQUEST {
        // Send a response
        let mut response = buf::NetBuffer::new();
        response.append_data(packet.payload());
        icmp_send(response, ICMP_ECHO_REPLY, source_ip);
    }
}

pub fn icmp_send(mut packet: buf::NetBuffer, packet_type: u8, dest_addr: util::IPv4Addr) {
    packet.add_header(ICMP_HEADER_LEN);
    let payload = packet.mut_payload();
    payload[0] = packet_type;
    let checksum = util::compute_checksum(payload);
    util::set_be16(&mut payload[2..4], checksum);
    ipv4::ip_send(packet, ipv4::PROTO_ICMP, dest_addr);
}
