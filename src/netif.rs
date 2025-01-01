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

#[repr(C)]
struct IOVec {
    base: *const u8,
    len: usize,
}

extern "C" {
    fn tun_init() -> i32;
    fn tun_recv(buffer: *mut u8, length: usize) -> i32;
    fn tun_sendv(vecs: *const IOVec, length: usize) -> i32;
}

static mut LOCAL_IP: util::IPv4Addr = util::IPv4Addr::new();

pub fn init() {
    unsafe {
        tun_init();
        LOCAL_IP = util::IPv4Addr::new_from(&[10, 0, 0, 2]);
    }
}

pub fn recv_packet() -> buf::NetBuffer {
    // Use a temporary buffer, which will result in an extra copy, but is
    // compatible with the NetBuffer API.
    // XXX Optimize this at some point.
    let mut readbuf = [0u8; 2048];
    let result = unsafe { tun_recv(readbuf.as_mut_ptr(), readbuf.len()) };
    if result <= 0 {
        println!("Error {} reading from TUN interface", result);
        std::process::exit(1);
    }

    let mut packet = buf::NetBuffer::new();
    packet.append_from_slice(&readbuf[..result as usize]);

    packet
}

pub fn send_packet(packet: buf::NetBuffer) {
    let mut writeVecs: Vec<IOVec> = Vec::new();
    for slice in packet.iter(usize::MAX) {
        writeVecs.push(IOVec {
            base: unsafe { slice.as_ptr() },
            len: slice.len(),
        });
    }

    let result = unsafe { tun_sendv(writeVecs.as_ptr() as *const IOVec, writeVecs.len()) };
    if result <= 0 {
        println!("Error {} writing to TUN interface", result);
        std::process::exit(1);
    }
}

pub fn get_ipaddr() -> util::IPv4Addr {
    unsafe { LOCAL_IP }
}
