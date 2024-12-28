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

///
/// Handling for buffering network data. This is functionally similar to the
/// mbuf structure in the BSD network stack, but reworked to be more idiomatic
/// in Rust, and with a object-oriented API.
///

use std::cmp;

const FRAG_SIZE: usize = 512;

type FragPointer = Option<Box<BufferFragment>>;

struct BufferFragment {
    data: [u8; FRAG_SIZE],
    data_start: usize,
    data_end: usize, // This is exclusive (past last byte of data)
    next: FragPointer,
}

pub struct NetBuffer {
    frags: FragPointer,
}

pub struct BufferIterator<'a> {
    current_frag: &'a FragPointer,
    skip: usize,
    remaining: usize,
}

impl BufferFragment {
    pub fn new() -> BufferFragment {
        BufferFragment {
            data: [0; FRAG_SIZE],
            data_start: 0,
            data_end: 0,
            next: None,
        }
    }

    pub fn len(&self) -> usize {
        self.data_end - self.data_start
    }
}

impl NetBuffer {
    pub fn new() -> NetBuffer {
        NetBuffer { frags: None }
    }

    /// XXX ideally we would keep a variable with the length and update that as
    /// we do mutating operations on the buffer. This would be faster in most
    /// cases, but a bit more complex, so I haven't implemented it yet.
    pub fn len(&self) -> usize {
        let mut len = 0;
        for frag in self.iter(0, usize::MAX) {
            len += frag.len();
        }

        len
    }

    /// Return an iterator that will walk through the frags in the buffer and
    /// return a slice for each.
    /// If offset is past the end of the buffer, the iterator will return None
    pub fn iter(&self, offset: usize, length: usize) -> BufferIterator {
        // Skip entire fragments if needed. This is necessary for correct
        // operation, as the iterator next() method relies on the first
        // fragment having something to copy.
        let mut current_frag = &self.frags;
        let mut skip = offset;
        while current_frag.is_some() && offset >= current_frag.as_ref().unwrap().len() {
            skip -= current_frag.as_ref().unwrap().len();
            current_frag = &current_frag.as_ref().unwrap().next;
        }

        BufferIterator {
            current_frag: current_frag,
            skip: skip,
            remaining: length,
        }
    }

    /// Return the initial frag of the buffer. This is used for reading
    /// header contents. Note: this slice be larger than the size returned
    /// by add_header.
    pub fn header(&self) -> &[u8] {
        assert!(self.frags.is_some()); // Shouldn't call on empty buffer
        let head_frag = self.frags.as_ref().unwrap();
        return &head_frag.data[head_frag.data_start..head_frag.data_end];
    }

    /// Same as header, but mutable. Used for writing the header.
    pub fn header_mut(&mut self) -> &mut [u8] {
        assert!(self.frags.is_some()); // Shouldn't call on empty buffer
        let head_frag = self.frags.as_mut().unwrap();
        return &mut head_frag.data[head_frag.data_start..head_frag.data_end];
    }

    /// Reserve space for another header to be prepended to the buffer
    /// (potentially in front of the headers for any encapsulated protocols).
    /// The network stack calls the protocol modules from highest to lowest
    /// layer, so this is called by each one as a packet is being prepared
    /// to send.
    ///
    /// This method guarantees the header is always contiguous (i.e. does not
    /// span multiple frags). The contents of the allocated space will be
    /// zeroed out.
    pub fn alloc_header(&mut self, size: usize) {
        if self.frags.is_none() || self.frags.as_ref().unwrap().data_start < size {
            // Prepend a new frag. We place the data at the end of the frag
            // to allow space for subsequent headers to be added.
            let mut new_head_frag = Box::new(BufferFragment::new());
            new_head_frag.data_start = FRAG_SIZE - size;
            new_head_frag.data_end = FRAG_SIZE;
            new_head_frag.next = if self.frags.is_none() {
                None
            } else {
                self.frags.take()
            };

            self.frags = Some(new_head_frag);
            return;
        }

        // There is sufficient space in the first frag to add the header.
        // Adjust the start of the frag head
        let frag = self.frags.as_mut().unwrap();
        frag.data_start -= size;

        // Zero out contents
        for i in 0..size {
            frag.data[frag.data_start + i] = 0;
        }
    }

    /// Remove space at beginning of buffer.
    pub fn trim_head(&mut self, size: usize) {
        let mut remaining = size;

        // Remove entire buffers if needed
        while remaining > 0 && self.frags.is_some() {
            let frag_len = self.frags.as_ref().unwrap().len();
            if frag_len > remaining {
                break;
            }

            remaining -= frag_len;
            self.frags = self.frags.as_mut().unwrap().next.take();
        }

        // Truncate the front buffer
        if remaining > 0 {
            let frag = self.frags.as_mut().unwrap();
            frag.data_start += remaining;
        }
    }

    pub fn append_from_slice(&mut self, data: &[u8]) {
        if data.len() == 0 {
            return;
        }

        // Find the last frag (or, if the buffer is empty, create a new one)
        let mut last_frag = if self.frags.is_none() {
            self.frags = Some(Box::new(BufferFragment::new()));
            &mut self.frags
        } else {
            let mut frag = &mut self.frags;
            while frag.as_mut().unwrap().next.is_some() {
                frag = &mut frag.as_mut().unwrap().next;
            }

            frag
        };

        let mut data_offset = 0;
        while data_offset < data.len() {
            let frag = last_frag.as_mut().unwrap();
            let copy_len = cmp::min(FRAG_SIZE - frag.data_end, data.len() - data_offset);
            frag.data[frag.data_end..frag.data_end + copy_len]
                .copy_from_slice(&data[data_offset..data_offset + copy_len]);
            frag.data_end += copy_len;
            data_offset += copy_len;
            if data_offset < data.len() {
                let new_frag = Some(Box::new(BufferFragment::new()));
                last_frag.as_mut().unwrap().next = new_frag;
                last_frag = &mut last_frag.as_mut().unwrap().next;
            }
        }
    }

    /// Opposite of append_from_slice, copy data out of the buffer.
    pub fn copy_to_slice(&self, length: usize, dest: &mut [u8]) {
        let mut copied = 0;
        let mut iter = self.iter(0, length);
        while copied < length {
            let next = iter.next();
            if next.is_none() {
                break;
            }

            let slice = next.unwrap();
            let copy_len = cmp::min(slice.len(), length - copied);
            dest[copied..copied + copy_len].copy_from_slice(&slice[..copy_len]);
            copied += copy_len;
        }
    }

    /// Copy data out of another buffer into this one.
    pub fn append_from_buffer(&mut self, other: &NetBuffer, length: usize) {
        for frag in other.iter(0, length) {
            self.append_from_slice(frag);
        }
    }

    /// This just takes over data from another buffer.
    pub fn append_buffer(&mut self, other: NetBuffer) {
        if self.frags.is_none() {
            self.frags = other.frags;
        } else {
            let mut last_frag = self.frags.as_mut().unwrap();
            while last_frag.next.is_some() {
                last_frag = last_frag.next.as_mut().unwrap();
            }

            last_frag.next = other.frags;
        }
    }

    /// Not sure if this will stay forever, but is convenient for now as it's closer to the
    /// existing API.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut vec = Vec::new();
        for frag in self.iter(0, usize::MAX) {
            vec.extend_from_slice(frag);
        }

        vec
    }
}

impl<'a> Iterator for BufferIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.current_frag.is_none() || self.remaining == 0 {
            return None;
        }

        // Note: we guarantee there is something in current_frag to be copied
        // The setup code iterates over frags that are entirely skipped.
        let frag = self.current_frag.as_ref().unwrap();
        let slice_length = cmp::min(frag.len() - self.skip, self.remaining);
        assert!(slice_length >= self.skip);
        assert!(self.remaining >= slice_length);
        let start_offs = frag.data_start + self.skip;
        let slice = &frag.data[start_offs..start_offs + slice_length];
        self.skip = 0;
        self.remaining -= slice_length;
        self.current_frag = &frag.next;
        Some(slice)
    }
}

mod tests {
    #[test]
    fn test_append() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        buf.append_from_slice(&[6, 7, 8, 9, 10]);
        buf.append_from_slice(&[11, 12, 13, 14, 15]);
        let mut dest = [0; 15];
        buf.copy_to_slice(15, &mut dest);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(buf.len(), 15);
    }

    #[test]
    fn test_grow_buffer() {
        let mut buf = super::NetBuffer::new();

        // Fill up the first frag
        let slice1 = [1; 512];
        buf.append_from_slice(&[1; 512]);
        assert_eq!(buf.len(), 512);

        // Add another frag
        let slice2 = [2; 512];
        buf.append_from_slice(&[2; 512]);
        assert_eq!(buf.len(), 1024);

        // Check the contents
        let mut dest = [0; 1024];
        buf.copy_to_slice(1024, &mut dest);
        assert_eq!(dest[..512], slice1);
        assert_eq!(dest[512..], slice2);
    }

    #[test]
    fn test_alloc_header() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.alloc_header(20);
        let mut dest = [0; 512];
        buf.copy_to_slice(512, &mut dest);
        assert_eq!(dest[..20], [0; 20]);
        assert_eq!(dest[20..], [1; 492]);
    }

    #[test]
    fn test_trim_head() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.trim_head(20);
        let mut dest = [0; 512];
        buf.copy_to_slice(512, &mut dest);
        assert_eq!(dest[..492], [1; 492]);
    }

    #[test]
    fn test_iter() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.append_from_slice(&[2; 512]);
        buf.append_from_slice(&[3; 512]);
        let mut iter = buf.iter(20, 1200);
        let slice1 = iter.next().unwrap();
        assert_eq!(slice1.len(), 492);
        assert_eq!(slice1[0], 1);
        let slice2 = iter.next().unwrap();
        assert_eq!(slice2.len(), 512);
        assert_eq!(slice2[0], 2);
        let slice3 = iter.next().unwrap();
        assert_eq!(slice3.len(), 196);
        assert_eq!(slice3[0], 3);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_to_vec() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.append_from_slice(&[2; 512]);
        buf.append_from_slice(&[3; 512]);
        let vec = buf.to_vec();
        assert_eq!(vec.len(), 1536);
        assert_eq!(vec[0..512], [1; 512]);
        assert_eq!(vec[512..1024], [2; 512]);
        assert_eq!(vec[1024..], [3; 512]);
    }

    #[test]
    fn test_append_buffer() {
        let mut buf1 = super::NetBuffer::new();
        buf1.append_from_slice(&[1; 512]);
        buf1.append_from_slice(&[2; 512]);
        buf1.append_from_slice(&[3; 512]);
        let mut buf2 = super::NetBuffer::new();
        buf2.append_from_slice(&[4; 512]);
        buf2.append_from_slice(&[5; 512]);
        buf2.append_from_slice(&[6; 512]);
        buf1.append_buffer(buf2);
        let mut dest = [0; 1536];
        buf1.copy_to_slice(1536, &mut dest);
        assert_eq!(dest[0..512], [1; 512]);
        assert_eq!(dest[512..1024], [2; 512]);
        assert_eq!(dest[1024..1536], [3; 512]);
        assert_eq!(buf1.len(), 3072);
    }

    #[test]
    fn test_append_from_buffer() {
        let mut buf1 = super::NetBuffer::new();
        buf1.append_from_slice(&[1; 512]);
        buf1.append_from_slice(&[2; 512]);
        buf1.append_from_slice(&[3; 512]);
        // 1536

        let mut buf2 = super::NetBuffer::new();
        buf2.append_from_slice(&[4; 512]);
        buf2.append_from_slice(&[5; 512]);
        buf2.append_from_slice(&[6; 512]);
        buf1.append_from_buffer(&buf2, 1000);

        let mut dest = [0; 3000];
        buf1.copy_to_slice(3000, &mut dest);
        assert_eq!(dest[0..512], [1; 512]);
        assert_eq!(dest[512..1024], [2; 512]);
        assert_eq!(dest[1024..1536], [3; 512]);
        assert_eq!(dest[1536..2048], [4; 512]);
        assert_eq!(dest[2048..2536], [5; 488]);

        assert_eq!(buf1.len(), 2536);
    }

    #[test]
    fn test_header() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.alloc_header(20);
        let header = buf.header();
        assert_eq!(header.len(), 20);
        assert_eq!(header[0], 0);
        assert_eq!(header[19], 0);
    }

    #[test]
    fn test_header_mut() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.alloc_header(20);
        let header = buf.header_mut();
        header[0] = 1;
        header[19] = 2;
        let header = buf.header();
        assert_eq!(header[0], 1);
        assert_eq!(header[19], 2);
    }

    #[test]
    fn test_grow_header() {
        let mut buf = super::NetBuffer::new();
        buf.alloc_header(20);
        {
            let header = buf.header_mut();
            header[0] = 1;
            header[19] = 2;
        }
        buf.alloc_header(20);
        {
            let header = buf.header_mut();
            header[0] = 3;
            header[19] = 4;
        }

        let header = buf.header();
        assert_eq!(header[0], 3);
        assert_eq!(header[19], 4);
        assert_eq!(header[20], 1);
        assert_eq!(header[39], 2);
    }

    #[test]
    fn test_trim_head_header1() {
        let mut buf = super::NetBuffer::new();
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        buf.trim_head(40);
        assert_eq!(buf.len(), 472);
        let header = buf.header();
        assert_eq!(header[0], 40);
        assert_eq!(header[5], 45);
    }

    #[test]
    fn test_trim_head_header2() {
        let mut buf = super::NetBuffer::new();
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        buf.alloc_header(20);
        buf.trim_head(40); // This will remove an entire fragment
        assert_eq!(buf.len(), 492);
        let header = buf.header();
        assert_eq!(header[0], 20);
        assert_eq!(header[5], 25);
    }

    #[test]
    fn test_trim_head_header3() {
        let mut buf = super::NetBuffer::new();
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        buf.alloc_header(20);
        buf.trim_head(10); // This will remove part of the header
        assert_eq!(buf.len(), 522);
    }
}
