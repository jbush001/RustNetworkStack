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
/// Handling for buffering network data.
/// This module provides a simple abstraction for temporarily storing any
/// data that is being sent or received by the network stack, including
/// the packets themselves or any queued receive or transmit data.
///
/// This is similar to how mbufs work in the BSD network stack (or, to a
/// lesser degree, skbuff in Linux)
/// This has two major design objectives:
/// 1. Reduce copies. This structure is used both to store packets and
///    queued data, which allows the former to be appended directly to the
///    latter without copying.
/// 2. Avoid fragmentation and optimize allocation. The primary allocation
///    unit is the fixed sizze BufferFragment, which can in theory be
///    allocated from a pool. This is not currently implemented.
///    Although the Box object does have an allocator parameter, it does not
///    seem to be supported well in Rust.
///
/// Alternatives:
/// - NetBuffer could also contain an array of pointers to fragments, which
///   would be more cache friendly and more idiomatic for Rust, but would limit
///   the maximum size of buffer (potentially workable for many protocols).
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
    fragments: FragPointer,
    length: usize,

    // XXX ideally this would also have a pointer to the tail frag, to avoid
    // having to walk the list to find it, but that's tricky given Rust's
    // ownership model.
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
        NetBuffer {
            fragments: None,
            length: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.length
    }

    /// Return an iterator that will walk through the fragments in the buffer and
    /// return a slice for each.
    /// If offset is past the end of the buffer, the iterator will return None
    pub fn iter(&self, offset: usize, length: usize) -> BufferIterator {
        // Skip entire fragments if needed. This is necessary for correct
        // operation, as the iterator next() method relies on the first
        // fragment having something to copy.
        let mut current_frag = &self.fragments;
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
        assert!(self.fragments.is_some()); // Shouldn't call on empty buffer
        let head_frag = self.fragments.as_ref().unwrap();
        return &head_frag.data[head_frag.data_start..head_frag.data_end];
    }

    /// Same as header, but mutable. Used for writing the header.
    pub fn header_mut(&mut self) -> &mut [u8] {
        assert!(self.fragments.is_some()); // Shouldn't call on empty buffer
        let head_frag = self.fragments.as_mut().unwrap();
        return &mut head_frag.data[head_frag.data_start..head_frag.data_end];
    }

    /// Reserve space for another header to be prepended to the buffer
    /// (potentially in front of the headers for any encapsulated protocols).
    /// The network stack calls the protocol modules from highest to lowest
    /// layer, so this is called by each one as a packet is being prepared
    /// to send.
    ///
    /// This method guarantees the header is always contiguous (i.e. does not
    /// span multiple fragments). The contents of the allocated space will be
    /// zeroed out.
    pub fn alloc_header(&mut self, size: usize) {
        assert!(size <= FRAG_SIZE);
        if self.fragments.is_none() || self.fragments.as_ref().unwrap().data_start < size {
            // Prepend a new frag. We place the data at the end of the frag
            // to allow space for subsequent headers to be added.
            let mut new_head_frag = Box::new(BufferFragment::new());
            new_head_frag.data_start = FRAG_SIZE - size;
            new_head_frag.data_end = FRAG_SIZE;
            new_head_frag.next = if self.fragments.is_none() {
                None
            } else {
                self.fragments.take()
            };

            self.fragments = Some(new_head_frag);
        } else {
            // There is sufficient space in the first frag to add the header.
            // Adjust the start of the frag head
            let frag = self.fragments.as_mut().unwrap();
            frag.data_start -= size;

            // Zero out contents
            for i in 0..size {
                frag.data[frag.data_start + i] = 0;
            }
        }

        self.length += size;
    }

    /// Remove space at beginning of buffer.
    pub fn trim_head(&mut self, size: usize) {
        assert!(size <= self.length);

        let mut remaining = size;

        // Remove entire buffers if needed
        while remaining > 0 && self.fragments.is_some() {
            let frag_len = self.fragments.as_ref().unwrap().len();
            if frag_len > remaining {
                break;
            }

            remaining -= frag_len;
            self.fragments = self.fragments.as_mut().unwrap().next.take();
        }

        // Truncate the front buffer
        if remaining > 0 {
            let frag = self.fragments.as_mut().unwrap();
            frag.data_start += remaining;
        }

        self.length -= size;
        assert!(self.fragments.is_some() || self.length == 0);
    }

    pub fn append_from_slice(&mut self, data: &[u8]) {
        if data.len() == 0 {
            return;
        }

        // Find the last frag (or, if the buffer is empty, create a new one)
        let mut last_frag = if self.fragments.is_none() {
            self.fragments = Some(Box::new(BufferFragment::new()));
            &mut self.fragments
        } else {
            let mut frag = &mut self.fragments;
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

        self.length += data.len();
    }

    /// Opposite of append_from_slice, copy data out of the buffer.
    pub fn copy_to_slice(&self, dest: &mut [u8], length: usize) -> usize {
        let mut copied = 0;
        let mut iter = self.iter(0, length);
        let to_copy = cmp::min(length, dest.len());
        while copied < to_copy {
            let next = iter.next();
            if next.is_none() {
                break;
            }

            let slice = next.unwrap();
            let copy_len = cmp::min(slice.len(), to_copy - copied);
            dest[copied..copied + copy_len].copy_from_slice(&slice[..copy_len]);
            copied += copy_len;
        }

        assert!(copied <= length);
        assert!(copied <= self.length);
        assert!(copied <= dest.len());
        assert!(copied == length || copied == self.length || copied == dest.len());
        assert!(copied != self.len() || iter.next().is_none());

        return copied;
    }

    /// Copy data out of another buffer into this one.
    pub fn append_from_buffer(&mut self, other: &NetBuffer, length: usize) {
        for frag in other.iter(0, length) {
            self.append_from_slice(frag);
        }
    }

    /// This just takes over data from another buffer.
    pub fn append_buffer(&mut self, other: NetBuffer) {
        self.length += other.length;
        if self.fragments.is_none() {
            self.fragments = other.fragments;
        } else {
            let mut last_frag = self.fragments.as_mut().unwrap();
            while last_frag.next.is_some() {
                last_frag = last_frag.next.as_mut().unwrap();
            }

            last_frag.next = other.fragments;
        }
    }
}

impl<'a> Iterator for BufferIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.current_frag.is_none() || self.remaining == 0 {
            return None;
        }

        // Note: we guarantee there is something in current_frag to be copied
        // The setup code iterates over fragments that are entirely skipped.
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
    fn test_append_from_slice() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);
        buf.append_from_slice(&[6, 7, 8, 9, 10]);
        assert_eq!(buf.len(), 10);
        buf.append_from_slice(&[11, 12, 13, 14, 15]);
        assert_eq!(buf.len(), 15);

        // Append an empty slice and ensure the length doesn't change.
        buf.append_from_slice(&[]);
        assert_eq!(buf.len(), 15);

        // Check contents
        let mut dest = [0; 15];
        buf.copy_to_slice(&mut dest, 15);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    }

    #[test]
    fn test_copy_to_slice1() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        // Try to copy fewer bytes than in the destination. Ensure it
        // doesn't overrun
        let mut dest = [0; 15];
        let copied = buf.copy_to_slice(&mut dest, 12);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0, 0, 0]);
        assert_eq!(buf.len(), 15);
        assert_eq!(copied, 12);
    }

    #[test]
    fn test_copy_to_slice2() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        // Try to copy more bytes than are in the buffer. Ensure it
        // returns a lesser count.
        let mut dest = [0; 15];
        let copied = buf.copy_to_slice(&mut dest, 20);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(copied, 15);
        assert_eq!(buf.len(), 15);
    }

    #[test]
    fn test_copy_to_slice3() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        // Try to copy more bytes than are in the destination.
        let mut dest = [0; 10];
        let copied = buf.copy_to_slice(&mut dest, 20);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(copied, 10);
    }

    #[test]
    fn test_grow_buffer() {
        let mut buf = super::NetBuffer::new();

        // Fill up the first frag
        let slice1 = [1; 500];
        buf.append_from_slice(&[1; 500]);
        assert_eq!(buf.len(), 500);

        // Add another frag. This will first fill the remainder of the
        // last frag, then add a new one.
        let slice2 = [2; 500];
        buf.append_from_slice(&[2; 500]);
        assert_eq!(buf.len(), 1000);

        // Check the contents
        let mut dest = [0; 1000];
        buf.copy_to_slice(&mut dest, 1000);
        assert_eq!(dest[..500], slice1);
        assert_eq!(dest[500..], slice2);
    }

    #[test]
    fn test_alloc_header() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        assert!(buf.len() == 512);

        // This will allocate a new fragment on the beginning of the chain.
        buf.alloc_header(20);
        assert!(buf.len() == 532);
        let mut dest = [0; 512];
        buf.copy_to_slice(&mut dest, 512);
        assert_eq!(dest[..20], [0; 20]);
        assert_eq!(dest[20..], [1; 492]);

        // This will add to the existing fragment
        buf.alloc_header(20);
        assert!(buf.len() == 552);
        let mut dest = [0; 512];
        buf.copy_to_slice(&mut dest, 512);
        assert_eq!(dest[..40], [0; 40]);
        assert_eq!(dest[40..], [1; 472]);
    }

    #[test]
    fn test_trim_head() {
        let mut buf = super::NetBuffer::new();
        // Create a slice with an incrementing count
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        assert!(buf.len() == 512);
        buf.trim_head(20);
        assert!(buf.len() == 492);

        // Check the contents
        let mut dest = [0; 512];
        buf.copy_to_slice(&mut dest, 512);
        for i in 0..492 {
            assert_eq!(dest[i], (i + 20) as u8);
        }
    }

    #[test]
    fn test_iter1() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.append_from_slice(&[2; 512]);
        buf.append_from_slice(&[3; 512]);

        // This range will chop both the first and last frag
        let mut iter = buf.iter(20, 1200);
        let slice1 = iter.next().unwrap();
        assert_eq!(slice1.len(), 492);
        assert_eq!(slice1[0], 1);
        assert_eq!(slice1[491], 1);
        let slice2 = iter.next().unwrap();
        assert_eq!(slice2.len(), 512);
        assert_eq!(slice2[0], 2);
        assert_eq!(slice2[511], 2);
        let slice3 = iter.next().unwrap();
        assert_eq!(slice3.len(), 196); // Not short buf
        assert_eq!(slice3[0], 3);
        assert_eq!(slice3[195], 3);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter2() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);

        // Zero length
        let mut iter = buf.iter(0, 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter3() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);

        // Offset past end of buffer
        let mut iter = buf.iter(513, 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_append_buffer() {
        let mut buf1 = super::NetBuffer::new();
        buf1.append_from_slice(&[1; 512]);
        buf1.append_from_slice(&[2; 512]);
        buf1.append_from_slice(&[3; 512]);
        assert!(buf1.len() == 1536);

        let mut buf2 = super::NetBuffer::new();
        buf2.append_from_slice(&[4; 512]);
        buf2.append_from_slice(&[5; 512]);
        buf2.append_from_slice(&[6; 512]);
        assert!(buf1.len() == 1536);

        buf1.append_buffer(buf2);
        assert!(buf1.len() == 3072);

        // Check contents
        let mut dest = [0; 1536];
        buf1.copy_to_slice(&mut dest, 1536);
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
        assert_eq!(buf1.len(), 1536);

        let mut buf2 = super::NetBuffer::new();
        buf2.append_from_slice(&[4; 512]);
        buf2.append_from_slice(&[5; 512]);
        buf2.append_from_slice(&[6; 512]);
        buf1.append_from_buffer(&buf2, 1000);
        assert_eq!(buf1.len(), 2536);

        // Check contents
        let mut dest = [0; 3000];
        buf1.copy_to_slice(&mut dest, 3000);
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
        header[0] = 0xcc;
        header[19] = 0x55;

        // copy out slices
        let mut dest = [0; 512];
        buf.copy_to_slice(&mut dest, 512);
        assert_eq!(dest[0], 0xcc);
        assert_eq!(dest[19], 0x55);
        assert_eq!(dest[20], 1);
    }

    #[test]
    fn test_grow_header() {
        let mut buf = super::NetBuffer::new();
        buf.alloc_header(20);
        assert_eq!(buf.len(), 20);
        {
            let header = buf.header_mut();
            header[0] = 1;
            header[19] = 2;
        }
        buf.alloc_header(20);
        assert_eq!(buf.len(), 40);
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
        assert_eq!(buf.len(), 512);
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
        assert_eq!(buf.len(), 512);
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
        assert_eq!(buf.len(), 512);
        buf.alloc_header(20);
        assert_eq!(buf.len(), 532);
        buf.trim_head(10); // This will remove part of the header
        assert_eq!(buf.len(), 522);
    }
}
