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

/// XXX This is meant to be a general purpose mechanism for storing network
/// data, including individual packets as well as queued data for a
/// connection. Ideally it could grow arbitrarily, but most implementations
/// of this sort of thing use linked lists extensively, which are at odds
/// with Rust's ownership model. So, for now, it's just hacked as a fixed
/// size buffer while I think about how to implement a more flexible version.

pub struct NetBuffer {
    data: [u8; 2048],
    length: usize,
    offset: usize,
}

const DEFAULT_HEADER_LEN: usize = 48;

impl NetBuffer {
    pub fn new() -> NetBuffer {
        NetBuffer {
            data: [0; 2048],
            // Reserve space for headers to avoid a copy later (see
            // add_header).
            offset: DEFAULT_HEADER_LEN,
            length: DEFAULT_HEADER_LEN,
        }
    }

    /// Returns a reference to the valid contents of the buffer (that
    /// is, what has been reserved by add_header or append_from_slice. This is
    /// used to read content of header fields.
    pub fn payload(&self) -> &[u8] {
        &self.data[self.offset..self.length]
    }

    /// Similar to payload, but returns a mutable reference. This is used on
    /// transmit to populate fields.
    pub fn mut_payload(&mut self) -> &mut [u8] {
        &mut self.data[self.offset..self.length]
    }

    /// Only used by the netif module
    pub fn start_read(&mut self) -> (*mut u8, usize) {
        self.offset = 0;
        (self.data.as_mut_ptr(), self.data.len())
    }

    /// Only used by the netif module
    pub fn end_read(&mut self, length: usize) {
        self.length = length;
    }

    /// Reserve space for another header to be prepended to the buffer
    /// (potentially in front of the headers for any encapsulated protocols).
    /// Subsequent calls to mut_payload will include this space in the
    /// beginning.
    /// The stack always invokes the protocol modules from highest to lowest
    /// layer, so this is called by each one as a packet is being prepared
    /// to send.
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
        }

        self.offset -= size;

        // Clear out the header
        for i in self.offset..self.offset + size {
            self.data[i] = 0u8;
        }
    }

    /// Remove space from a header. This is the opposite of add_header.
    /// The protocol modules are called in order from lowest to highest layer,
    /// With each one processing its own header, removing it, then calling
    /// into the next layer up (encapsulated) to process the next header.
    pub fn remove_header(&mut self, size: usize) {
        assert!(self.offset + size < self.length);
        self.offset += size;
    }

    /// Add data to the end of the buffer. This is generally only called by
    /// the innermost/highest protocol layer to populate the data contents
    /// of the packet.
    pub fn append_from_slice(&mut self, data: &[u8]) {
        assert!(self.length + data.len() < self.data.len());
        self.data[self.offset..self.offset + data.len()].copy_from_slice(data);
        self.length += data.len();
    }
}

mod tests {

    use crate::buf::NetBuffer;

    #[test]
    fn test_add_header() {
        let mut buf = NetBuffer::new();
        buf.append_from_slice([0xaa, 0xbb, 0xcc, 0xdd, 0xee].as_slice());
        {
            let payload = buf.payload();
            assert!(payload.len() == 5);
            assert!(payload[0] == 0xaa);
            assert!(payload[1] == 0xbb);
            assert!(payload[2] == 0xcc);
            assert!(payload[3] == 0xdd);
            assert!(payload[4] == 0xee);
        }

        buf.add_header(10);
        {
            let payload = buf.mut_payload();
            assert!(payload.len() == 15);
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
            let payload = buf.payload();
            assert!(payload.len() == 105);
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
