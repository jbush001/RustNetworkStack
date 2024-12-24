use crate::packet;
use crate::icmp;
use crate::util;

#[repr(C)]
struct IPHeader {
    version_ihl : u8,
    tos: u8,
    total_length: u16,
    id: u16,
    flags_frag: u16,
    ttl: u8,
    proto: u8,
    checksum: u16,
    source_addr: u32,
    dest_addr: u32,
}

const PROTO_ICMP: u8 = 1;

fn get_ip_header(pkt: &mut packet::NetworkPacket) -> &IPHeader {
    let header = unsafe {
        &*(pkt.data.as_ptr().add(pkt.offset as usize) as *const IPHeader)
    };

    pkt.offset += std::mem::size_of::<IPHeader>() as i32;
    header
}

pub fn ip_recv(pkt: &mut packet::NetworkPacket) {
    let checksum = util::compute_checksum(&pkt.data[pkt.offset as usize..pkt.length as usize]);

    let header = get_ip_header(pkt);
    if (header.version_ihl >> 4) != 4 {
        return; // Not IPV4
    }

    println!("version {:02x}", header.version_ihl >> 4);
    println!("Checksum: {:04x}", checksum);
    println!("total length {:04x}", u16::from_be(header.total_length));
    println!("protocol {:02x}", header.proto);
    println!("source addr {:08x}", u32::from_be(header.source_addr));
    println!("dest addr {:08x}", u32::from_be(header.dest_addr));
    let source_addr = u32::from_be(header.source_addr);

    if header.proto == PROTO_ICMP {
        icmp::icmp_recv(pkt, source_addr);
    }
}
