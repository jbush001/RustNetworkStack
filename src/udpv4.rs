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
use crate::ipv4;

const UDP_HEADER_LEN: usize = 8;

//
//    0               1               2               3
//    +-------------------------------+-------------------------------+
//  0 |         Source Port           |          Dest Port            |
//    +-------------------------------+-------------------------------+
//  4 |            Length             |           Checksum            |
//    +-------------------------------+-------------------------------+
//


pub fn udp_recv(mut packet: buf::NetBuffer, source_addr: util::IPv4Addr) {
    println!("Got UDP packet");

    let payload = packet.payload();
    let source_port = util::get_be16(&payload[0..2]);
    let dest_port = util::get_be16(&payload[2..4]);
    let length = util::get_be16(&payload[4..6]);
    packet.remove_header(UDP_HEADER_LEN);

    println!("Source port {} dest port {}", source_port, dest_port);
    println!("Length {}", length);

    // XXX hack: respond to packet
    udp_send(packet, source_addr, dest_port, source_port);
}

fn udp_send(mut packet: buf::NetBuffer, dest_addr: util::IPv4Addr, source_port: u16, dest_port: u16) {
    packet.add_header(UDP_HEADER_LEN);
    let length = packet.payload_len() as u16;
    let payload = packet.mut_payload();
    util::set_be16(&mut payload[0..2], source_port);
    util::set_be16(&mut payload[2..4], dest_port);
    util::set_be16(&mut payload[4..6], length);
    util::set_be16(&mut payload[6..8], 0); // Skip computing checksum

    ipv4::ip_send(packet, ipv4::PROTO_UDP, dest_addr);
}
