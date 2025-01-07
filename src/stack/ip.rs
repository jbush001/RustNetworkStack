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
use crate::icmp;
use crate::netif;
use crate::tcp;
use crate::udp;
use crate::util;
use std::sync::atomic::{AtomicU16, Ordering};

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

const IP_HEADER_LEN: usize = 20;
static mut NEXT_PACKET_ID: AtomicU16 = AtomicU16::new(0);
const DEFAULT_TTL: u8 = 64;

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

pub fn ip_input(mut packet: buf::NetBuffer) {
    // A common way to decode packet headers is to cast the raw byte
    // array to a packed structure. This is a bit more challenging in
    // Rust (it's sketchy in any language, but Rust is more of a stickler).
    // Instead, I manually decode the relevant fields into local variables.
    let header = packet.header();
    let version = header[0] >> 4;
    if version != 4 {
        println!("IP: Unsupported version");
        return;
    }

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

    match protocol {
        PROTO_ICMP => icmp::icmp_input(packet, source_addr),
        PROTO_TCP => tcp::tcp_input(packet, source_addr),
        PROTO_UDP => udp::udp_input(packet, source_addr),
        _ => println!("IP: Unknown protocol {}", protocol),
    }
}

pub fn ip_output(mut packet: buf::NetBuffer, protocol: u8, dest_addr: util::IPAddr) {
    packet.alloc_header(IP_HEADER_LEN);
    let packet_length = packet.len() as u16;
    let header = packet.header_mut();

    header[0] = 0x45; // Version/IHL
    util::set_be16(&mut header[2..4], packet_length); // Total Length

    util::set_be16(
        &mut header[4..6], // ID
        unsafe { NEXT_PACKET_ID.fetch_add(1, Ordering::AcqRel) },
    );

    header[8] = DEFAULT_TTL; // TTL
    header[9] = protocol; // Protocol
    netif::get_ipaddr().copy_to(&mut header[12..16]); // Source Address
    dest_addr.copy_to(&mut header[16..20]); // Destination Address

    let checksum = util::compute_checksum(&header[..IP_HEADER_LEN]);
    util::set_be16(&mut header[10..12], checksum);

    netif::send_packet(packet);
}
