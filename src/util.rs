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

use crate::buf;
use std::convert::TryInto;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IPv4Addr {
    addr: [u8; 4],
}

impl IPv4Addr {
    pub const fn new() -> Self {
        Self { addr: [0; 4] }
    }

    pub fn new_from(addr: &[u8]) -> Self {
        Self {
            addr: addr.try_into().unwrap(),
        }
    }

    pub fn copy_to(&self, buffer: &mut [u8]) {
        buffer.copy_from_slice(&self.addr);
    }
}

impl std::fmt::Display for IPv4Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.addr[0], self.addr[1], self.addr[2], self.addr[3]
        )
    }
}

// Compute one's complement sum, per RFV 1071
// https://datatracker.ietf.org/doc/html/rfc1071
pub fn compute_ones_comp(in_checksum: u16, slice: &[u8]) -> u16 {
    let mut checksum: u32 = in_checksum as u32;

    let mut i = 0;
    while i < slice.len() - 1 {
        checksum += u16::from_be_bytes([slice[i], slice[i + 1]]) as u32;
        i += 2;
    }

    if i < slice.len() {
        checksum += slice[i] as u32;
    }

    while checksum > 0xffff {
        checksum = (checksum & 0xffff) + (checksum >> 16);
    }

    checksum as u16
}

pub fn compute_checksum(slice: &[u8]) -> u16 {
    0xffff ^ compute_ones_comp(0, slice)
}

pub fn compute_buffer_ones_comp(initial_sum: u16, buffer: &buf::NetBuffer) -> u16 {
    let mut sum = initial_sum;
    for frag in buffer.iter(usize::MAX) {
        sum = compute_ones_comp(sum, frag);
    }

    sum
}

pub fn get_be16(buffer: &[u8]) -> u16 {
    ((buffer[0] as u16) << 8) | buffer[1] as u16
}

pub fn get_be32(buffer: &[u8]) -> u32 {
    ((buffer[0] as u32) << 24)
        | ((buffer[1] as u32) << 16)
        | ((buffer[2] as u32) << 8)
        | buffer[3] as u32
}

pub fn set_be16(buffer: &mut [u8], value: u16) {
    buffer[0] = ((value >> 8) & 0xff) as u8;
    buffer[1] = (value & 0xff) as u8;
}

pub fn set_be32(buffer: &mut [u8], value: u32) {
    buffer[0] = ((value >> 24) & 0xff) as u8;
    buffer[1] = ((value >> 16) & 0xff) as u8;
    buffer[2] = ((value >> 8) & 0xff) as u8;
    buffer[3] = (value & 0xff) as u8;
}

pub fn print_binary(buffer: &[u8]) {
    for (i, byte) in buffer.iter().enumerate() {
        print!("{:02x} ", byte);
        if i % 16 == 15 {
            println!();
        }
    }

    println!();
}

pub fn seq_gt(val1: u32, val2: u32) -> bool {
    let diff = val1.wrapping_sub(val2);
    diff < 0x80000000 && diff != 0
}

pub struct PerfCounter(AtomicU32);

impl PerfCounter {
    pub const fn new() -> Self {
        PerfCounter(AtomicU32::new(0))
    }

    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(&self, value: u32) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

pub struct Statistics {
    pub packets_received: PerfCounter,
    pub packets_sent: PerfCounter,
    pub packets_retransmitted: PerfCounter,
    pub buffers_allocated: PerfCounter,
    pub buffers_freed: PerfCounter,
    pub buffers_created: PerfCounter,
}

pub static STATS: Statistics = Statistics {
    packets_received: PerfCounter::new(),
    packets_sent: PerfCounter::new(),
    packets_retransmitted: PerfCounter::new(),
    buffers_allocated: PerfCounter::new(),
    buffers_freed: PerfCounter::new(),
    buffers_created: PerfCounter::new(),
};

pub fn print_stats() {
    println!("Packets received: {}", STATS.packets_received.get());
    println!("Packets sent: {}", STATS.packets_sent.get());
    println!("Packets retransmitted: {}", STATS.packets_retransmitted.get());
    println!("Buffers allocated: {}", STATS.buffers_allocated.get());
    println!("Buffers freed: {}", STATS.buffers_freed.get());
    println!("Buffers created: {}", STATS.buffers_created.get());

    let current_buf_inuse = STATS.buffers_allocated.get() - STATS.buffers_freed.get();
    let current_memory = buf::buffer_count_to_memory(current_buf_inuse);
    let total_buffer_memory = buf::buffer_count_to_memory(STATS.buffers_created.get());
    println!("Current buffer memory in use: {}k", current_memory / 1024);
    println!("Total buffer memory allocated: {}k", total_buffer_memory / 1024);
}

mod tests {
    #[test]
    fn test_compute_ones_comp() {
        assert_eq!(super::compute_ones_comp(0, &[0x00, 0x00]), 0);
        assert_eq!(super::compute_ones_comp(0, &[0x00, 0x01]), 0x1);
        assert_eq!(super::compute_ones_comp(0, &[0x00, 0xff]), 0xff);
        assert_eq!(
            super::compute_ones_comp(0, &[0xff, 0x23, 0xef, 0x55]),
            0xee79
        );
    }

    #[test]
    fn test_compute_checksum() {
        assert_eq!(super::compute_checksum(&[0x00, 0x00]), 0xffff);
        assert_eq!(super::compute_checksum(&[0x00, 0x01]), 0xfffe);
        assert_eq!(super::compute_checksum(&[0x00, 0xff]), 0xff00);
        assert_eq!(super::compute_checksum(&[0xff, 0x23, 0xef, 0x55]), 0x1186);
    }

    #[test]
    fn test_compute_packet_ones_comp() {
        let mut buffer = crate::buf::NetBuffer::new();
        buffer.append_from_slice(&[0x12, 0x34]);
        assert_eq!(super::compute_buffer_ones_comp(0, &buffer), 0x1234);
    }

    #[test]
    fn test_compute_packet_ones_comp_multiple_fragments() {
        let mut buffer = crate::buf::NetBuffer::new();
        for _ in 0..512 {
            buffer.append_from_slice(&[0x12, 0x34]);
        }

        // 512 * 0x1234 = 0x246800, 0x6800 + 0x0024 = 0x6824

        assert_eq!(super::compute_buffer_ones_comp(0, &buffer), 0x6824);
    }

    #[test]
    fn test_compute_ones_comp_odd_length() {
        assert_eq!(super::compute_ones_comp(0, &[0x12, 0x34, 0x56]), 0x128a);
    }

    #[test]
    fn test_get_be16() {
        assert_eq!(super::get_be16(&[0x00, 0x00]), 0x0000);
        assert_eq!(super::get_be16(&[0x35, 0xa5]), 0x35a5);
    }

    #[test]
    fn test_get_be32() {
        assert_eq!(super::get_be32(&[0xde, 0xad, 0xbe, 0xef]), 0xdeadbeef);
        assert_eq!(super::get_be32(&[0x00, 0x00, 0x00, 0x01]), 0x00000001);
        assert_eq!(super::get_be32(&[0x00, 0x00, 0x00, 0xff]), 0x000000ff);
        assert_eq!(super::get_be32(&[0x00, 0x00, 0xff, 0x00]), 0x0000ff00);
        assert_eq!(super::get_be32(&[0x00, 0xff, 0x00, 0x00]), 0x00ff0000);
        assert_eq!(super::get_be32(&[0xff, 0x00, 0x00, 0x00]), 0xff000000);
    }

    #[test]
    fn test_set_be16() {
        let mut buffer = [0u8; 2];
        super::set_be16(&mut buffer, 0x0000);
        assert_eq!(buffer, [0x00, 0x00]);
        super::set_be16(&mut buffer, 0x0001);
        assert_eq!(buffer, [0x00, 0x01]);
        super::set_be16(&mut buffer, 0x00ff);
        assert_eq!(buffer, [0x00, 0xff]);
        super::set_be16(&mut buffer, 0x0100);
        assert_eq!(buffer, [0x01, 0x00]);
        super::set_be16(&mut buffer, 0xffff);
        assert_eq!(buffer, [0xff, 0xff]);
    }

    #[test]
    fn test_set_be32() {
        let mut buffer = [0u8; 4];
        super::set_be32(&mut buffer, 0x00000000);
        assert_eq!(buffer, [0x00, 0x00, 0x00, 0x00]);
        super::set_be32(&mut buffer, 0x00000001);
        assert_eq!(buffer, [0x00, 0x00, 0x00, 0x01]);
        super::set_be32(&mut buffer, 0x000000ff);
        assert_eq!(buffer, [0x00, 0x00, 0x00, 0xff]);
        super::set_be32(&mut buffer, 0x00000100);
        assert_eq!(buffer, [0x00, 0x00, 0x01, 0x00]);
        super::set_be32(&mut buffer, 0x0000ffff);
        assert_eq!(buffer, [0x00, 0x00, 0xff, 0xff]);
        super::set_be32(&mut buffer, 0x00010000);
        assert_eq!(buffer, [0x00, 0x01, 0x00, 0x00]);
        super::set_be32(&mut buffer, 0x00ffffff);
        assert_eq!(buffer, [0x00, 0xff, 0xff, 0xff]);
        super::set_be32(&mut buffer, 0x01000000);
        assert_eq!(buffer, [0x01, 0x00, 0x00, 0x00]);
        super::set_be32(&mut buffer, 0xdeadbeef);
        assert_eq!(buffer, [0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn test_ip_to_str() {
        assert_eq!(
            super::IPv4Addr::new_from(&[18u8, 52, 86, 120]).to_string(),
            "18.52.86.120"
        );
    }

    #[test]
    fn test_copy_to() {
        let ip = super::IPv4Addr::new_from(&[192, 168, 1, 1]);
        let mut buffer = [0u8; 4];
        ip.copy_to(&mut buffer);
        assert_eq!(buffer, [192, 168, 1, 1]);
    }

    #[test]
    fn test_seq_gt() {
        assert_eq!(super::seq_gt(0x00000001, 0x00000000), true);
        assert_eq!(super::seq_gt(0x00000000, 0x00000001), false);
        assert_eq!(super::seq_gt(0x7fffffff, 0x80000000), false);
        assert_eq!(super::seq_gt(0x80000000, 0x7fffffff), true);
        assert_eq!(super::seq_gt(21, 21), false);
    }
}
