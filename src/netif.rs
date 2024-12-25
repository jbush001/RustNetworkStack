// Wrappers for the C functions in tun.c

use crate::packet;
use crate::util;

extern {
    fn tun_init() -> i32;
    fn tun_recv(buffer: *mut u8, length: i32) -> i32;
    fn tun_send(buffer: *const u8, length: i32) -> i32;
}

const LOCAL_IP: [u8; 4] = [10, 0, 0, 2];
static mut ip_packed: u32 = 0;

pub fn init() {
    unsafe {
        tun_init();
        ip_packed = util::get_be32(&LOCAL_IP[0..4]);
    }
}

pub fn recv_packet() -> packet::NetworkPacket {
    let mut pkt = packet::alloc();
    unsafe {
        pkt.length = tun_recv(pkt.data.as_mut_ptr(), pkt.data.len() as i32) as u32;
    }

    pkt
}

pub fn send_packet(pkt: packet::NetworkPacket) {
    unsafe {
        tun_send(pkt.data.as_ptr().add(pkt.offset as usize), (pkt.length - pkt.offset) as i32);
    }
}

pub fn get_ipaddr() -> u32 {
    return unsafe { ip_packed };
}
