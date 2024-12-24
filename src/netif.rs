// Wrappers for the C functions in tun.c

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

pub fn recv_packet(buffer: &mut [u8]) -> i32 {
    unsafe {
        tun_recv(buffer.as_mut_ptr(), buffer.len() as i32)
    }
}

pub fn send_packet(buffer: &[u8]) -> i32 {
    unsafe {
        tun_send(buffer.as_ptr(), buffer.len() as i32)
    }
}
