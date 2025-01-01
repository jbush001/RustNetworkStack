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

    // We eschew all type checking and just pass the iovecs as raw byte pointers.
    fn tun_recv(vecs: *const u8, length: usize) -> i32;
    fn tun_send(vecs: *const u8, length: usize) -> i32;
}

static mut LOCAL_IP: util::IPv4Addr = util::IPv4Addr::new();

pub fn init() {
    unsafe {
        tun_init();
        LOCAL_IP = util::IPv4Addr::new_from(&[10, 0, 0, 2]);
    }
}

fn to_iovec(packet: &buf::NetBuffer) -> Vec<IOVec> {
    let mut iovec: Vec<IOVec> = Vec::new();
    for slice in packet.iter(usize::MAX) {
        iovec.push(IOVec {
            base: slice.as_ptr(),
            len: slice.len(),
        });
    }

    iovec
}

pub fn recv_packet() -> buf::NetBuffer {
    let mut packet = buf::NetBuffer::new();
    packet.preallocate(2048);
    let iovec = to_iovec(&packet);
    let result = unsafe { tun_recv(iovec.as_ptr() as *const u8, iovec.len()) };
    if result <= 0 {
        println!("Error {} reading from TUN interface", result);
        std::process::exit(1);
    }

    packet.truncate_to_size(result as usize);

    packet
}

pub fn send_packet(packet: buf::NetBuffer) {
    let iovec = to_iovec(&packet);
    let result = unsafe { tun_send(iovec.as_ptr() as *const u8, iovec.len()) };
    if result <= 0 {
        println!("Error {} writing to TUN interface", result);
        std::process::exit(1);
    }
}

pub fn get_ipaddr() -> util::IPv4Addr {
    unsafe { LOCAL_IP }
}
