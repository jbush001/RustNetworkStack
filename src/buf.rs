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

pub fn alloc() -> NetBuffer {
    NetBuffer {
        data: [0; 2048],
        length: 0,
        offset: 0
    }
}

impl NetBuffer {
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
                std::ptr::copy(self.data.as_ptr(), self.data.as_mut_ptr().add(grow_size),
                    self.length - self.offset);
            }
            self.offset += grow_size;
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
}
