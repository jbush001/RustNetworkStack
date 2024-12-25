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
use crate::tcp;
use crate::util;
use crate::netif;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;

const IP_HEADER_LEN: u32 = 20;
static mut next_packet_id: u16 = 0;
const DEFAULT_TTL : u8 = 64;


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

pub fn ip_recv(mut packet: buf::NetBuffer) {
    let payload = packet.payload();
    let version = (payload[0] as u8) >> 4;
    if version != 4 {
        return;
    }

    let header_len = (((payload[0] as u8) & 0xf) as u32) * 4;
    let checksum = util::compute_checksum(&payload[..header_len as usize]);
    if checksum != 0 {
        println!("IP checksum error {:04x}", checksum);
        return;
    }

    let protocol = payload[9] as u8;
    let source_addr = util::get_be32(&payload[12..16]);
    let dest_addr = util::get_be32(&payload[16..20]);

    println!("Version {}", version);
    println!("Protocol {}", protocol);
    println!("Source addr {}", util::ip_to_str(source_addr));
    println!("Dest addr {}", util::ip_to_str(dest_addr));
    packet.offset += header_len;

    match protocol {
        PROTO_ICMP => icmp::icmp_recv(packet, source_addr),
        PROTO_TCP => tcp::tcp_recv(packet, source_addr),
        _ => println!("Unkonwn protocol {}", protocol)
    }
}

pub fn ip_send(mut packet: buf::NetBuffer, protocol: u8, dest_addr: util::IPv4Addr) {
    assert!(packet.offset > IP_HEADER_LEN);
    packet.offset -= IP_HEADER_LEN;
    let packet_length = packet.payload_len() as u16;
    let payload = packet.mut_payload();

    payload[0] = 0x45; // Version/IHL
    util::set_be16(&mut payload[2..4], packet_length); // Total Length

    unsafe {
        util::set_be16(&mut payload[4..6], next_packet_id); // ID
        next_packet_id += 1;
    }

    payload[8] = DEFAULT_TTL; // TTL
    payload[9] = protocol; // Protocol
    util::set_be32(&mut payload[12..16], netif::get_ipaddr()); // Source Address
    util::set_be32(&mut payload[16..24], dest_addr); // Destination Address

    let checksum = util::compute_checksum(payload);
    util::set_be16(&mut payload[10..12], checksum);

    netif::send_packet(packet);
}
