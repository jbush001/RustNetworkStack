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

// Internet Protocol as described in RFC 791

use crate::buf;
use crate::icmp;
use crate::netif;
use crate::tcp;
use crate::udp;
use crate::util;
use std::sync::atomic::{AtomicU16, Ordering};

pub const PROTO_ICMPV4: u8 = 1;
pub const PROTO_ICMPV6: u8 = 58;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

const IPV4_BASE_HEADER_LEN: usize = 20;
const IPV6_HEADER_LEN: usize = 40;

static NEXT_PACKET_ID: AtomicU16 = AtomicU16::new(0);
const DEFAULT_TTL: u8 = 64;

pub fn ip_input(packet: buf::NetBuffer) {
    let header = packet.header();
    let version = header[0] >> 4;
    if version == 4 {
        ip_input_v4(packet);
    } else if version == 6 {
        ip_input_v6(packet);
    } else {
        println!("IP: Invalid version field");
    }
}

//    0               1               2               3
//    +-------+-------+---------------+-------------------------------+
//  0 |Version|  IHL  |Type of Service|          Total Length         |
//    +-------+-------+---------------+-----+-------------------------+
//  4 |         Identification        |Flags|      Fragment Offset    |
//    +---------------+---------------+-----+-------------------------+
//  8 |  Time to Live |    Protocol   |         Header Checksum       |
//    +---------------+---------------+-------------------------------+
// 12 |                       Source Address                          |
//    +---------------------------------------------------------------+
// 16 |                    Destination Address                        |
//    +-----------------------------------------------+---------------+
// 20 |                    Options                    |    Padding    |
//    +-----------------------------------------------+---------------+

fn ip_input_v4(mut packet: buf::NetBuffer) {
    // A common way to decode packet headers is to cast the raw byte
    // array to a packed structure. This is a bit more challenging in
    // Rust (it's sketchy in any language, but Rust is more of a stickler).
    // Instead, I manually decode the relevant fields into local variables.
    let header = packet.header();
    let header_len = ((header[0] & 0xf) as usize) * 4;

    // Note that we don't decode IP options here, but just skip them.
    // These are generally not used.

    let checksum = util::compute_checksum(&header[..header_len]);
    if checksum != 0 {
        println!("IP checksum error {:04x}", checksum);
        return;
    }

    // Reassembing fragmented packet is not supported, but this seems
    // to be very rare.
    if (util::get_be16(&header[6..8]) & 0x3fff) != 0 {
        println!("IP: Fragmented packet, not supported");
        return;
    }

    let protocol = header[9];
    let source_addr = util::IPAddr::new_from(&header[12..16]);

    packet.trim_head(header_len);
    ip_input_common(packet, protocol, source_addr);
}

//
//    0               1               2               3
//    +------------+-------------------+------------------------------+
//  0 | Version(4) | Traffic Class (8) |      Flow Label (16)         |
//    +------------+-------------------+----------------+-------------+
//  4 |      Payload Length (16)       | Next Header(8) | Hop Limit   |
//    +--------------------------------+----------------+-------------+
//  8 |                                                               |
//    |                       Source Address                          |
//    |                                                               |
//    |                                                               |
//    +---------------------------------------------------------------+
// 24 |                                                               |
//    |                    Destination Address                        |
//    |                                                               |
//    |                                                               |
//    +---------------------------------------------------------------+

fn ip_input_v6(mut packet: buf::NetBuffer) {
    let header = packet.header();
    let protocol = header[6];
    let source_addr = util::IPAddr::new_from(&header[8..24]);

    // No IP header checksum...

    packet.trim_head(IPV6_HEADER_LEN);
    ip_input_common(packet, protocol, source_addr);
}

fn ip_input_common(packet: buf::NetBuffer, protocol: u8, source_addr: util::IPAddr) {
    match protocol {
        PROTO_ICMPV4 => icmp::icmp_input_v4(packet, source_addr),
        PROTO_ICMPV6 => icmp::icmp_input_v6(packet, source_addr),
        PROTO_TCP => tcp::tcp_input(packet, source_addr),
        PROTO_UDP => udp::udp_input(packet, source_addr),
        _ => println!("IP: Unknown protocol {}", protocol),
    }
}

pub fn ip_output(packet: buf::NetBuffer, protocol: u8, dest_addr: util::IPAddr) {
    match dest_addr {
        util::IPAddr::V4(_) => ip_output_v4(packet, protocol, dest_addr),
        util::IPAddr::V6(_) => ip_output_v6(packet, protocol, dest_addr),
    }
}

fn ip_output_v4(mut packet: buf::NetBuffer, protocol: u8, dest_addr: util::IPAddr) {
    packet.alloc_header(IPV4_BASE_HEADER_LEN);
    let packet_length = packet.len() as u16;
    let header = packet.header_mut();

    header[0] = 0x45; // Version/IHL
    util::set_be16(&mut header[2..4], packet_length); // Total Length

    util::set_be16(
        &mut header[4..6], // ID
        NEXT_PACKET_ID.fetch_add(1, Ordering::AcqRel),
    );

    header[8] = DEFAULT_TTL; // TTL
    header[9] = protocol; // Protocol
    netif::get_ipaddr().0.copy_to(&mut header[12..16]); // Source Address
    dest_addr.copy_to(&mut header[16..20]); // Destination Address

    let checksum = util::compute_checksum(&header[..IPV4_BASE_HEADER_LEN]);
    util::set_be16(&mut header[10..12], checksum);

    netif::send_packet(packet);
}

fn ip_output_v6(mut packet: buf::NetBuffer, protocol: u8, dest_addr: util::IPAddr) {
    let payload_length = packet.len() as u16;
    packet.alloc_header(IPV6_HEADER_LEN);

    let header = packet.header_mut();
    header[0] = 0x60; // Version/traffic class/flow label
    util::set_be16(&mut header[4..6], payload_length); // Payload length
    header[6] = protocol; // Next header
    header[7] = DEFAULT_TTL; // Hop limit
    netif::get_ipaddr().1.copy_to(&mut header[8..24]); // Source address
    dest_addr.copy_to(&mut header[24..40]); // Destination address

    netif::send_packet(packet);
}
