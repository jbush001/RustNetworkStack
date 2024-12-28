//
// Copyright 2024 Jeff Bush
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

mod buf;
mod icmpv4;
mod ipv4;
mod netif;
mod tcpv4;
mod udpv4;
mod util;

use std::io::Read;
use std::thread::sleep;
use std::time::Duration;

fn packet_receive_thread() {
    loop {
        let packet = netif::recv_packet();
        ipv4::ip_input(packet);
    }
}

fn test_udp_echo() {
    let mut socket = udpv4::udp_open(8000);
    loop {
        let mut source_addr: util::IPv4Addr = 0;
        let mut source_port: u16 = 0;
        let mut data = [0; 1500];

        let received = udpv4::udp_recv(&mut socket, &mut data, &mut source_addr, &mut source_port);
        println!(
            "Received UDP packet from {}:{} ({} bytes)",
            source_addr,
            source_port,
            received
        );

        util::print_binary(&data[..received as usize]);
        udpv4::udp_send(&mut socket, source_addr, source_port, &data[..received as usize]);
    }
}

fn test_tcp_connect() {
    // XXX Give a little time to start tcpdump
    // std::thread::sleep(std::time::Duration::from_secs(5));

    let mut handle = tcpv4::tcp_open(0x0a000001, 3000);
    if handle.is_none() {
        println!("Failed to open socket");
        return;
    }

    let mut socket = handle.unwrap();

    println!("Socket is open");

    const REQUEST_STRING: &str = "GET / HTTP/1.0\r\n\r\n";
    const REQUEST_BYTES: &[u8] = REQUEST_STRING.as_bytes();
    tcpv4::tcp_write(&mut socket, REQUEST_BYTES);
    loop {
        sleep(Duration::from_millis(100));
        let mut data = [0; 1500];
        let received = tcpv4::tcp_read(&mut socket, &mut data);
        if received > 0 {
            print!("{}", std::str::from_utf8(&data[..received as usize]).unwrap());
        }
    }
}

fn main() {
    netif::init();
    std::thread::spawn(move || {
        packet_receive_thread();
    });

    std::thread::spawn(move || {
        test_udp_echo();
    });

    // Wait for a key press
    println!("Press key to connect");
    let _ = std::io::stdin().read(&mut [0u8]).unwrap();

    test_tcp_connect();
    std::thread::park();
}
