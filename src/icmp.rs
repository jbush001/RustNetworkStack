use crate::packet;
use crate::util;

//    0               1               2               3
//    +---------------+---------------+-----+-------------------------+
//  0 |     Type      |     Code      |          Checksum             |
//    +---------------+---------------+-------------------------------+
//  4 |                     Payload...                                |
//    +---------------------------------------------------------------+


const ICMP_ECHO_REQUEST: u8 = 8;

pub fn icmp_recv(pkt: packet::NetworkPacket, source_ip: u32) {
    let payload = &pkt.data[pkt.offset as usize..pkt.length as usize];
    let checksum = util::compute_checksum(&payload);
    if checksum != 0 {
        print!("ICMP checksum error");
        return;
    }

    let packet_type = payload[0];
    if packet_type == ICMP_ECHO_REQUEST {
        println!("echo request from {:4x}", source_ip);
        // XXX Send a response
    }
}

