use crate::packet;
use crate::util;

#[repr(C)]
struct ICMPHeader {
    pkttype: u8,
    code: u8,
    checksum: u16
}

const ICMP_ECHO_REQUEST: u8 = 8;

fn get_icmp_header(pkt: &mut packet::NetworkPacket) -> &ICMPHeader {
    let header = unsafe {
        &*(pkt.data.as_ptr().add(pkt.offset as usize) as *const ICMPHeader)
    };

    pkt.offset += std::mem::size_of::<ICMPHeader>() as i32;
    header
}

pub fn icmp_recv(pkt: &mut packet::NetworkPacket, source_ip: u32) {
    let checksum = util::compute_checksum(&pkt.data[pkt.offset as usize..pkt.length as usize]);
    let header = get_icmp_header(pkt);

    println!("icmp_recv");
    println!("type = {:02x}", header.pkttype);
    println!("code = {:02x}", header.code);
    println!("checksum = {:04x}", checksum);

    if header.pkttype == ICMP_ECHO_REQUEST {
        println!("echo request from {:4x}", source_ip);
        // XXX Send a response
    }
}
