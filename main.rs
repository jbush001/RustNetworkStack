extern {
    fn tun_init();
    fn tun_recv(buffer: *mut u8, length: i32) -> i32;
    fn tun_send(buffer: *const u8, length: i32) -> i32;
}

fn print_binary(buffer: &[u8]) {
    for byte in buffer {
        print!("{:02x} ", byte);
    }
    println!();
}

fn main() {
    unsafe {
        tun_init();
    }
    let mut buffer = [0u8; 2048];

    loop  {
        unsafe {
            let length = tun_recv(buffer.as_mut_ptr(), buffer.len() as i32);
            if length < 0 {
                break;
            }

            print_binary(&buffer[..length as usize]);
        }
    }
}
