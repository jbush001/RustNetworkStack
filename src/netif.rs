// Wrappers for the C functions in tun.c

use crate::packet;

extern {
    fn tun_init(remote_ip_addr: *const u8) -> i32;
    fn tun_recv(buffer: *mut u8, length: i32) -> i32;
    fn tun_send(buffer: *const u8, length: i32) -> i32;
}

const REMOTE_IP : [u8; 4] = [10, 0, 0, 1];

pub fn init() {
    unsafe {
        tun_init(REMOTE_IP.as_ptr());
    }
}

pub fn recv_packet() -> packet::NetworkPacket {
    let mut pkt = packet::NetworkPacket {
        data: [0; 2048],
        length: 0
    };

    unsafe {
        pkt.length = tun_recv(pkt.data.as_mut_ptr(), pkt.data.len() as i32);
    }

    pkt
}

pub fn send_packet(pkt: packet::NetworkPacket) {
    unsafe {
        tun_send(pkt.data.as_ptr(), pkt.length as i32);
    }
}
