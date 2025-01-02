//
// Copyright 2024-2025 Jeff Bush
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

use netstack::{init_netstack, udpv4, util};

fn main() {
    init_netstack();

    let result = udpv4::udp_open(8000);
    if result.is_err() {
        println!("Failed to open socket: {}", result.err().unwrap());
        return;
    }

    let mut socket = result.unwrap();

    loop {
        let mut source_addr: util::IPv4Addr = util::IPv4Addr::new();
        let mut source_port: u16 = 0;
        let mut data = [0; 1500];

        let received = udpv4::udp_recv(&mut socket, &mut data, &mut source_addr, &mut source_port);
        println!(
            "Received UDP packet from {}:{} ({} bytes)",
            source_addr, source_port, received
        );

        util::print_binary(&data[..received as usize]);
        udpv4::udp_send(
            &mut socket,
            source_addr,
            source_port,
            &data[..received as usize],
        );

        util::print_stats();
    }
}

