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

pub type IPv4Addr = u32;

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
    for frag in buffer.iter(0, usize::MAX) {
        sum = compute_ones_comp(sum, frag);
    }

    sum
}

pub fn get_be16(buffer: &[u8]) -> u16 {
    (((buffer[0] as u16) << 8) | buffer[1] as u16) as u16
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

pub fn ip_to_str(addr: IPv4Addr) -> String {
    format!(
        "{}.{}.{}.{}",
        (addr >> 24) & 0xff,
        (addr >> 16) & 0xff,
        (addr >> 8) & 0xff,
        addr & 0xff
    )
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
    diff < 0x80000000
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
        assert_eq!(super::ip_to_str(0x12345678), "18.52.86.120");
    }

    #[test]
    fn test_compute_packet_checksum() {
        let mut buffer = crate::buf::NetBuffer::new();
        buffer.append_from_slice(&[0x12, 0x34]);
        assert_eq!(super::compute_buffer_ones_comp(0, &buffer), 0x1234);
    }

    #[test]
    fn test_seq_gt() {
        assert_eq!(super::seq_gt(0x00000001, 0x00000000), true);
        assert_eq!(super::seq_gt(0x00000000, 0x00000001), false);
        assert_eq!(super::seq_gt(0x7fffffff, 0x80000000), false);
        assert_eq!(super::seq_gt(0x80000000, 0x7fffffff), true);
    }
}
