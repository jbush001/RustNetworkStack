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

use std::io::Read;
use std::time::Duration;
use netstack::{init_netstack, tcp, util};
use std::thread::sleep;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let ipv6 = args.len() > 1 && args[1] == "v6";

    init_netstack();

    // Wait for a key press
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

    const REQUEST_STRING: &str = "GET / HTTP/1.0\r\n\r\n";
    const REQUEST_BYTES: &[u8] = REQUEST_STRING.as_bytes();
    tcp::tcp_write(&mut socket, REQUEST_BYTES);
    for _ in 0..25 {
        sleep(Duration::from_millis(100));
        let mut data = [0; 1500];
        let received = tcp::tcp_read(&mut socket, &mut data);
        if received < 0 {
            println!("Connection closed");
            break;
        }
        if received > 0 {
            print!(
                "{}",
                std::str::from_utf8(&data[..received as usize]).unwrap()
            );
        }
    }

    println!("Closing socket");
    tcp::tcp_close(&mut socket);
    std::mem::drop(socket);

    // Wait a spell to see what other things come in.
    sleep(Duration::from_secs(5));

    util::print_metrics();
}
