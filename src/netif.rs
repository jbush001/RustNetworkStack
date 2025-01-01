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

const MAX_VECS: usize = 8;

#[derive(Copy, Clone)]
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

fn to_iovec(packet: &buf::NetBuffer, vec: &mut [IOVec]) -> usize {
    let mut vec_count = 0;
    for slice in packet.iter(usize::MAX) {
        vec[vec_count] = IOVec {
            base: slice.as_ptr(),
            len: slice.len(),
        };

        vec_count += 1
    }

    vec_count
}

pub fn recv_packet() -> buf::NetBuffer {
    const MRU: usize = 2048;
    let mut packet = buf::NetBuffer::new_prealloc(MRU);
    let mut iovec: [IOVec; MAX_VECS] = [IOVec{
        base: 0 as *const u8,
        len: 0,
    }; MAX_VECS];
    let num_vecs = to_iovec(&packet, iovec.as_mut_slice());
    let result = unsafe { tun_recv(iovec.as_ptr() as *const u8, num_vecs) };
    if result <= 0 {
        println!("Error {} reading from TUN interface", result);
        std::process::exit(1);
    }

    packet.trim_tail(packet.len() - result as usize);

    packet
}

pub fn send_packet(packet: buf::NetBuffer) {
    let mut iovec: [IOVec; MAX_VECS] = [IOVec{
        base: 0 as *const u8,
        len: 0,
    }; MAX_VECS];
    let num_vecs = to_iovec(&packet, iovec.as_mut_slice());
    let result = unsafe { tun_send(iovec.as_ptr() as *const u8, num_vecs) };
    if result <= 0 {
        println!("Error {} writing to TUN interface", result);
        std::process::exit(1);
    }
}

pub fn get_ipaddr() -> util::IPv4Addr {
    unsafe { LOCAL_IP }
}
