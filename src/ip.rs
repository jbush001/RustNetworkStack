use crate::packet;
use crate::icmp;
use crate::util;
use crate::netif;

pub const PROTO_ICMP: u8 = 1;
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

pub fn ip_recv(mut pkt: packet::NetworkPacket) {
    let payload = &pkt.data[pkt.offset as usize..pkt.length as usize];
    let version = (payload[0] as u8) >> 4;
    if version != 4 {
        return;
    }

    let checksum = util::compute_checksum(&payload);
    if checksum != 0 {
        print!("IP checksum error");
        return;
    }

    let header_len = ((payload[0] as u8) & 0xf) as u32;
    pkt.offset += header_len * 4;
    let protocol = payload[9] as u8;
    let source_addr = util::get_be32(&payload[12..16]);
    let dest_addr = util::get_be32(&payload[16..20]);

    println!("Version {}", version);
    println!("Protocol {}", protocol);
    println!("Source addr {}", util::ip_to_str(source_addr));
    println!("Dest addr {}", util::ip_to_str(dest_addr));

    if protocol == PROTO_ICMP {
        icmp::icmp_recv(pkt, source_addr);
    }
}

pub fn ip_send(mut pkt: packet::NetworkPacket, protocol: u8, dest_addr: u32) {
    assert!(pkt.offset > IP_HEADER_LEN);
    pkt.offset -= IP_HEADER_LEN;
    let payload = &mut pkt.data[pkt.offset as usize..pkt.length as usize];

    payload[0] = 0x45; // Version/IHL
    util::set_be16(&mut payload[2..4], (pkt.length - pkt.offset) as u16); // Total Length

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

    netif::send_packet(pkt);
}
