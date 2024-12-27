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

fn packet_receive_thread() {
    loop {
        let packet = netif::recv_packet();
        println!("Received buf ({} bytes):", packet.length);
        util::print_binary(packet.payload());
        ipv4::ip_input(packet);
    }
}

fn test_udp_echo() {
    let mut socket = udpv4::udp_open(8000);
    loop {
        let (source_addr, source_port, data) = udpv4::udp_recv(&mut socket);
        println!(
            "Received UDP packet from {}:{} ({} bytes)",
            source_addr,
            source_port,
            data.len()
        );
        util::print_binary(&data);
        udpv4::udp_send(&mut socket, source_addr, source_port, &data);
    }
}

fn test_tcp_connect() {
    // XXX Give a little time to start tcpdump
    // std::thread::sleep(std::time::Duration::from_secs(5));

    let mut socket = tcpv4::tcp_open(0x0a000001, 8765);
}

fn main() {
    netif::init();
    std::thread::spawn(move || {
        packet_receive_thread();
    });

    std::thread::spawn(move || {
        test_udp_echo();
    });

    test_tcp_connect();
    std::thread::park();
}
