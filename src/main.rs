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

fn main() {
    netif::init();

    let socket = udpv4::UDPSocket::new(8000);
    loop {
        let packet = netif::recv_packet();
        println!("Received buf ({} bytes):", packet.length);
        util::print_binary(packet.payload());
        ipv4::ip_recv(packet);

        let mut sock_guard = socket.lock().unwrap();
        let recv = sock_guard.receive();
        if recv.is_some() {
            let (source_addr, source_port, data) = recv.unwrap();
            println!("Received UDP packet from {}:{} ({} bytes)", source_addr, source_port, data.len());
            util::print_binary(&data);
            sock_guard.send(source_addr, source_port, &data);
        }
    }
}
