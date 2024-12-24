use crate::packet;
use crate::icmp;
use crate::util;

const PROTO_ICMP: u8 = 1;


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

    let header_len = ((payload[0] as u8) & 0xf) as i32;
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
