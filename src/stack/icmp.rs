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
use crate::ip;
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

pub fn icmp_input(mut packet: buf::NetBuffer, source_ip: util::IPAddr) {
    let header = packet.header();
    let checksum = util::compute_buffer_ones_comp(0, &packet) ^ 0xffff;
    if checksum != 0 {
        print!("ICMP checksum error");
        return;
    }

    let packet_type = header[0];
    packet.trim_head(ICMP_HEADER_LEN);
    if packet_type == ICMP_ECHO_REQUEST {
        // Send a response
        let mut response = buf::NetBuffer::new();
        response.append_from_buffer(&packet, usize::MAX);
        icmp_output(response, ICMP_ECHO_REPLY, source_ip);
    }
}

pub fn icmp_output(mut packet: buf::NetBuffer, packet_type: u8, dest_addr: util::IPAddr) {
    packet.alloc_header(ICMP_HEADER_LEN);
    let header = packet.header_mut();
    header[0] = packet_type;
    let checksum = util::compute_buffer_ones_comp(0, &packet) ^ 0xffff;

    let header = packet.header_mut();
    util::set_be16(&mut header[2..4], checksum);
    ip::ip_output(packet, ip::PROTO_ICMP, dest_addr);
}
