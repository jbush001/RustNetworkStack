//
// Copyright 2025 Jeff Bush
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

use netstack::{init_netstack, tcp};

const PORT: u16 = 8080;

fn main() {
    init_netstack();
    let mut listen_sock = tcp::tcp_listen(PORT);
    if listen_sock.is_err() {
        println!("Failed to open socket: {}", listen_sock.err().unwrap());
        return;
    }

    println!("Listening on port {}", PORT);

    loop {
        let result = tcp::tcp_accept(listen_sock.as_mut().unwrap());
        if result.is_err() {
            println!("Failed to accept connection: {}", result.err().unwrap());
            return;
        }

        let mut socket = result.unwrap();
        println!("Accepted connection");

        let mut data = [0; 1500];
        let received = tcp::tcp_read(&mut socket, &mut data);
        if received < 0 {
            println!("Connection closed");
            continue;
        }

        let request = std::str::from_utf8(&data[..received as usize]);
        if request.is_err() {
            println!("Failed to parse request");
            continue;
        }

        println!("Received request: {}", request.unwrap());

        const RESPONSE: &str = "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Hello, world!</h1></body></html>";
        tcp::tcp_write(&mut socket, RESPONSE.as_bytes());
        tcp::tcp_close(&mut socket);
    }
}
