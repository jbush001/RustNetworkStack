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

// Wrappers for the C functions in tun.c

use crate::buf;
use crate::util;

extern "C" {
    fn tun_init() -> i32;
    fn tun_recv(buffer: *mut u8, length: usize) -> i32;
    fn tun_send(buffer: *const u8, length: usize) -> i32;
}

const LOCAL_IP: util::IPv4Addr = 0x0a000002; // 10.0.0.2

pub fn init() {
    unsafe {
        tun_init();
    }
}

pub fn recv_packet() -> buf::NetBuffer {
    let mut packet = buf::NetBuffer::new();
    packet.offset = 0;
    unsafe {
        let result = tun_recv(packet.data.as_mut_ptr(), packet.data.len());
        if result <= 0 {
            println!("Error {} reading from TUN interface", result);
            std::process::exit(1);
        }

        packet.length = result as usize;
    }

    packet
}

pub fn send_packet(packet: buf::NetBuffer) {
    unsafe {
        let result = tun_send(
            packet.data.as_ptr().add(packet.offset),
            packet.payload_len(),
        );
        if result <= 0 {
            println!("Error {} writing to TUN interface", result);
            std::process::exit(1);
        }
    }
}

pub fn get_ipaddr() -> util::IPv4Addr {
    return LOCAL_IP;
}
