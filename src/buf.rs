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
    pub length: u32,
    pub offset: u32,
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
        &self.data[self.offset as usize..self.length as usize]
    }

    pub fn mut_payload(&mut self) -> &mut [u8] {
        &mut self.data[self.offset as usize..self.length as usize]
    }

    pub fn payload_len(&self) -> usize {
        (self.length - self.offset) as usize
    }
}
