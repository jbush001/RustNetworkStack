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

use netstack::{init_netstack, tcp, util};
use std::env;
use std::io::Read;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    let ipv6 = args.len() > 1 && args[1] == "6v";

    init_netstack();

    println!("Press key to connect");
    let _ = std::io::stdin().read(&mut [0u8]).unwrap();

    let addr = if ipv6 {
        util::IPAddr::new_from(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x1])
    } else {
        util::IPAddr::new_from(&[10, 0, 0, 1])
    };

    let result = tcp::tcp_open(addr, 3000);
    if result.is_err() {
        println!("Failed to open socket: {}", result.err().unwrap());
        return;
    }

    let mut socket = result.unwrap();

    println!("Socket is open");
    let mut data = [0; 0x100000];

    // Write a chargen pattern into the buffer
    // Each line is 72 ASCII characters along with a CR/LF
    let line_length = 74;
    let pattern_length = 95;
    for (i, elem) in data.iter_mut().enumerate() {
        let line_num = i / line_length;
        let line_offset = i % line_length;
        let start_char = line_num % pattern_length;

        *elem = if line_offset == line_length - 2 {
            b'\r'
        } else if line_offset == line_length - 1 {
            b'\n'
        } else {
            ((start_char + line_offset) % pattern_length + 32) as u8
        };
    }

    tcp::tcp_write(&mut socket, &data);

    println!("Closing socket");
    tcp::tcp_close(&mut socket);
    std::mem::drop(socket);

    // Wait a spell to see what other things come in.
    sleep(Duration::from_secs(5));

    util::print_metrics();
}
