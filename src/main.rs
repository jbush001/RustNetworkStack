mod netif;

fn print_binary(buffer: &[u8]) {
    for (i, byte) in buffer.iter().enumerate() {
        print!("{:02x} ", byte);
        if i % 16 == 15 {
            println!();
        }
    }

    println!();
}

fn main() {
    netif::init();

    let mut buffer = [0u8; 2048];

    loop  {
        let length = netif::recv_packet(&mut buffer);
        if length < 0 {
            break;
        }

        println!("Received packet ({} bytes):", length);
        print_binary(&buffer[..length as usize]);
    }
}
