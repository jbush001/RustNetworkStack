mod netif;
mod packet;

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

    loop  {
        let pkt = netif::recv_packet();
        println!("Received packet ({} bytes):", pkt.length);
        print_binary(&pkt.data[..pkt.length as usize]);
    }
}
