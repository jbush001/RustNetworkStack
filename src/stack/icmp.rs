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

// Internet Control Message Protocol, as described in RFC 792 and RFC 4443

// XXX This should send errors to the higher layer protocols
// Right now it only supports pings.

use crate::buf;
use crate::ip;
use crate::util;
use crate::netif;

// The header has the same layout for V4 and V6, but the type codes are
// different.
//
//    0               1               2               3
//    +---------------+---------------+-----+-------------------------+
//  0 |     Type      |     Code      |          Checksum             |
//    +---------------+---------------+-------------------------------+
//  4 |                        Payload...                             |
//    +---------------------------------------------------------------+

const ICMPV4_ECHO_REQUEST: u8 = 8;
const ICMPV4_ECHO_REPLY: u8 = 0;
const ICMPV6_ECHO_REQUEST: u8 = 128;
const ICMPV6_ECHO_REPLY: u8 = 129;

const ICMP_HEADER_LEN: usize = 4;

pub fn icmp_input_v4(mut packet: buf::NetBuffer, source_ip: util::IPAddr) {
    let header = packet.header();
    let checksum = util::compute_buffer_ones_comp(0, &packet) ^ 0xffff;
    if checksum != 0 {
        println!("ICMPv4 checksum error");
        return;
    }

    let packet_type = header[0];
    packet.trim_head(ICMP_HEADER_LEN);
    if packet_type == ICMPV4_ECHO_REQUEST {
        // Send a response
        let mut response = buf::NetBuffer::new();
        response.append_from_buffer(&packet, usize::MAX);
        icmp_output_v4(response, ICMPV4_ECHO_REPLY, source_ip);
    }
}

pub fn icmp_input_v6(mut packet: buf::NetBuffer, source_ip: util::IPAddr) {
    let ph_checksum = util::compute_pseudo_header_checksum(
        source_ip,
        netif::get_ipaddr().1,
        packet.len(),
        ip::PROTO_ICMPV6,
    );

    let header = packet.header();
    let checksum = util::compute_buffer_ones_comp(ph_checksum, &packet) ^ 0xffff;
    if checksum != 0 {
        println!("ICMPv6 checksum error");
        return;
    }

    let packet_type = header[0];
    packet.trim_head(ICMP_HEADER_LEN);
    if packet_type == ICMPV6_ECHO_REQUEST {
        // Send a response
        let mut response = buf::NetBuffer::new();
        response.append_from_buffer(&packet, usize::MAX);
        icmp_output_v6(response, ICMPV6_ECHO_REPLY, source_ip);
    }
}

pub fn icmp_output_v4(mut packet: buf::NetBuffer, packet_type: u8, dest_addr: util::IPAddr) {
    packet.alloc_header(ICMP_HEADER_LEN);
    let header = packet.header_mut();
    header[0] = packet_type;
    let checksum = util::compute_buffer_ones_comp(0, &packet) ^ 0xffff;

    let header = packet.header_mut();
    util::set_be16(&mut header[2..4], checksum);
    ip::ip_output(packet, ip::PROTO_ICMPV4, dest_addr);
}

pub fn icmp_output_v6(mut packet: buf::NetBuffer, packet_type: u8, dest_addr: util::IPAddr) {
    packet.alloc_header(ICMP_HEADER_LEN);
    let header = packet.header_mut();
    header[0] = packet_type;

    let ph_checksum = util::compute_pseudo_header_checksum(
        netif::get_ipaddr().1,
        dest_addr,
        packet.len(),
        ip::PROTO_ICMPV6,
    );

    let checksum = util::compute_buffer_ones_comp(ph_checksum, &packet) ^ 0xffff;
    let header = packet.header_mut();
    util::set_be16(&mut header[2..4], checksum);
    ip::ip_output(packet, ip::PROTO_ICMPV6, dest_addr);
}