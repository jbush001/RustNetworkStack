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
use crate::icmpv4;
use crate::netif;
use crate::tcpv4;
use crate::udpv4;
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
    let header = packet.header();
    let version = (header[0] as u8) >> 4;
    if version != 4 {
        return;
    }

    let header_len = (((header[0] as u8) & 0xf) as usize) * 4;
    let checksum = util::compute_checksum(&header[..header_len]);
    if checksum != 0 {
        println!("IP checksum error {:04x}", checksum);
        return;
    }

    let protocol = header[9] as u8;
    let source_addr = util::get_be32(&header[12..16]);
    let dest_addr = util::get_be32(&header[16..20]);

    println!("Version {}", version);
    println!("Protocol {}", protocol);
    println!("Source addr {}", util::ip_to_str(source_addr));
    println!("Dest addr {}", util::ip_to_str(dest_addr));
    packet.trim_head(header_len);

    match protocol {
        PROTO_ICMP => icmpv4::icmp_input(packet, source_addr),
        PROTO_TCP => tcpv4::tcp_input(packet, source_addr),
        PROTO_UDP => udpv4::udp_input(packet, source_addr),
        _ => println!("Unkonwn protocol {}", protocol),
    }
}

pub fn ip_output(mut packet: buf::NetBuffer, protocol: u8, dest_addr: util::IPv4Addr) {
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
    util::set_be32(&mut header[12..16], netif::get_ipaddr()); // Source Address
    util::set_be32(&mut header[16..24], dest_addr); // Destination Address

    let checksum = util::compute_checksum(&header[..IP_HEADER_LEN]);
    util::set_be16(&mut header[10..12], checksum);

    netif::send_packet(packet);
}
