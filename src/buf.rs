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

pub struct NetBuffer {
    pub data: [u8; 2048],
    pub length: usize,
    pub offset: usize,
}

const DEFAULT_HEADER_LEN: usize = 48;

impl NetBuffer {
    pub fn new() -> NetBuffer {
        NetBuffer {
            data: [0; 2048],
            offset: DEFAULT_HEADER_LEN, // Reserve space for headers
            length: DEFAULT_HEADER_LEN,
        }
    }

    pub fn payload(&self) -> &[u8] {
        &self.data[self.offset..self.length]
    }

    pub fn mut_payload(&mut self) -> &mut [u8] {
        &mut self.data[self.offset..self.length]
    }

    pub fn payload_len(&self) -> usize {
        self.length - self.offset
    }

    pub fn add_header(&mut self, size: usize) {
        if self.offset < size {
            // Grow the buffer to give space to prepend a new header
            // (plus a little more space)
            let grow_size = size + 32;
            unsafe {
                std::ptr::copy(
                    self.data.as_ptr().add(self.offset),
                    self.data.as_mut_ptr().add(self.offset + grow_size),
                    self.length - self.offset,
                );
            }
            self.offset += grow_size;
            self.length += grow_size;
            println!("Grow buffer");
        }

        self.offset -= size;

        // Clear out the header
        for i in self.offset..self.offset + size {
            self.data[i] = 0u8;
        }
    }

    pub fn remove_header(&mut self, size: usize) {
        assert!(self.offset + size < self.length);
        self.offset += size;
    }

    pub fn append_from_slice(&mut self, data: &[u8]) {
        assert!(self.length + data.len() < self.data.len());
        self.data[self.offset..self.offset + data.len()].copy_from_slice(data);
        self.length += data.len();
    }
}

mod tests {
    #[test]
    fn test_add_header() {
        let mut buf = NetBuffer::new();
        buf.append_from_slice([0xaa, 0xbb, 0xcc, 0xdd, 0xee].as_slice());
        {
            assert!(buf.payload_len() == 5);
            let payload = buf.payload();
            assert!(payload[0] == 0xaa);
            assert!(payload[1] == 0xbb);
            assert!(payload[2] == 0xcc);
            assert!(payload[3] == 0xdd);
            assert!(payload[4] == 0xee);
        }

        buf.add_header(10);
        {
            assert!(buf.payload_len() == 15);
            let payload = buf.mut_payload();
            // Check that the payload is zeroed out
            assert!(payload[0] == 0);
            assert!(payload[9] == 0);

            // We'll check these later
            payload[0] = 0x55;
            payload[9] = 0x66;

            // Ensure the previous values are still valid.
            assert!(payload[10] == 0xaa);
            assert!(payload[11] == 0xbb);
            assert!(payload[12] == 0xcc);
            assert!(payload[13] == 0xdd);
            assert!(payload[14] == 0xee);
        }

        // Force a copy
        buf.add_header(90);
        {
            assert!(buf.payload_len() == 105);
            let payload = buf.payload();
            assert!(payload[90] == 0x55);
            assert!(payload[99] == 0x66);
            assert!(payload[100] == 0xaa);
            assert!(payload[101] == 0xbb);
            assert!(payload[102] == 0xcc);
            assert!(payload[103] == 0xdd);
            assert!(payload[104] == 0xee);
        }
    }
}
