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
/// This class implements an efficient, flexible container for unstructured
/// data, which is used as temporary storage throughout the network stack,
/// including packets and queued receive and transmit data. The design is
/// similar to mbufs in the BSD network stack or skbuff in Linux, although
/// this is implemented more idiomatically with Rust's ownership model.
///
/// The design goals of any network buffering system are to minimize copies,
/// avoid external heap fragmentation, and optimize allocation speed. The
/// base storage unit is a fixed-size BufferFragment. These fragments are
/// chained together to allow buffers to grow to arbitrary sizes. Fragments
/// are allocated from a fixed pool of memory, which is fast.
///
/// Alternatives:
/// - I also considered having NetBuffer contain an array of pointers
///   to fragments, which would be simpler and potentially more cache friendly,
///   but would limit the maximum size of buffer (potentially workable for
///   many protocols).
///

use std::cmp;
use std::sync::Mutex;

const FRAGMENT_SIZE: usize = 512;
type FragPointer = Option<Box<BufferFragment>>;

// This is the publicly visible abstraction for clients of this API.
pub struct NetBuffer {
    fragments: FragPointer, // Head of linked list of fragments

    // This is always equal the sum of the lengths of fragments.
    // (data_end - data_start for each). I maintain this separately to
    // speed up calls to get the length.
    length: usize,

    // XXX ideally this would also have a pointer to the tail frag, to avoid
    // having to walk the list to find it, but that's tricky given Rust's
    // ownership model.
}

/// Portion of a buffer, which represents a node in a linked list.
struct BufferFragment {
    data: [u8; FRAGMENT_SIZE],
    data_start: usize, // Offset into data array of first valid byte of data.
    data_end: usize,   // Same for last. This is exclusive (one past last byte)
    next: FragPointer, // Next fragment in linked list.
}

pub struct BufferIterator<'a> {
    current_frag: &'a FragPointer,
    remaining: usize, // How many more bytes to copy.
}

const GROW_SIZE: usize = 16;
static g_free_list: Mutex<FragPointer> = Mutex::new(FragPointer::None);
static mut g_pool_size: usize = 0;
static mut g_free_bufs: usize = 0;
static mut g_total_allocs: u64 = 0;

fn alloc_fragment() -> Box<BufferFragment> {
    let mut free_list = g_free_list.lock().unwrap();
    if free_list.is_none() {
        // Grow the pool.
        for _ in 0..GROW_SIZE {
            let mut frag = Box::new(BufferFragment::new());
            frag.next = free_list.take();
            *free_list = Some(frag);
        }

        unsafe {
            g_pool_size += GROW_SIZE;
            g_free_bufs += GROW_SIZE;
        }
    }

    unsafe {
        assert!(g_free_bufs > 0);
        g_total_allocs += 1;
        g_free_bufs -= 1;
    }

    let mut new_frag = free_list.take().unwrap();
    new_frag.data_start = 0;
    new_frag.data_end = 0;
    new_frag.next = None;

    new_frag
}

/// Put a fragment back into the pool.
/// It's still necessary to explicitly return these (vs having them
/// automatically return when they go out of scope). The Box class does
/// have an allocator parameter, but it is marked as unstable and not fully
/// supported.
fn free_fragment(mut fragment: Box<BufferFragment>) {
    let mut free_list = g_free_list.lock().unwrap();
    unsafe { g_free_bufs += 1; }
    fragment.next = free_list.take();
    *free_list = Some(fragment);
}

pub fn print_alloc_stats() {
    unsafe {
        println!("Total pool size: {} ({}k)", g_pool_size, g_pool_size * FRAGMENT_SIZE / 1024);
        println!("Free buffers: {} ({}k)", g_free_bufs, g_free_bufs * FRAGMENT_SIZE / 1024);
        println!("Total allocations: {}", g_total_allocs);
    }
}

impl BufferFragment {
    pub fn new() -> BufferFragment {
        BufferFragment {
            data: [0; FRAGMENT_SIZE],
            data_start: 0,
            data_end: 0,
            next: None,
        }
    }

    pub fn len(&self) -> usize {
        self.data_end - self.data_start
    }
}

impl Drop for NetBuffer {
    fn drop(&mut self) {
        let mut frag = self.fragments.take();
        while frag.is_some() {
            let next = frag.as_mut().unwrap().next.take();
            free_fragment(frag.unwrap());
            frag = next;
        }
    }
}

impl NetBuffer {
    pub fn new() -> NetBuffer {
        NetBuffer {
            fragments: None,
            length: 0,
        }
    }

    /// Return the total available data within this buffer.
    pub fn len(&self) -> usize {
        self.length
    }

    /// Return an iterator that will walk through the fragments in the buffer and
    /// return a slice for each.
    pub fn iter(&self, length: usize) -> BufferIterator {
        BufferIterator {
            current_frag: &self.fragments,
            remaining: length,
        }
    }

    /// Return a slice pointing to data in the initial fragment of the buffer.
    /// This is used for reading header contents. Note: this slice may be larger
    /// than the size returned by add_header.
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
        assert!(size <= FRAGMENT_SIZE);
        if self.fragments.is_none() || self.fragments.as_ref().unwrap().data_start < size {
            // Prepend a new frag. We place the data at the end of the frag
            // to allow space for subsequent headers to be added.
            let mut new_head_frag = alloc_fragment();
            new_head_frag.data_start = FRAGMENT_SIZE - size;
            new_head_frag.data_end = FRAGMENT_SIZE;
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
            frag.data[frag.data_start..frag.data_start + size].fill(0);
        }

        self.length += size;
    }

    /// Remove data from the beginning of buffer.
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
            let mut dead_frag = self.fragments.take().unwrap();
            self.fragments = dead_frag.next.take();
            free_fragment(dead_frag);
        }

        // Truncate the front buffer
        if remaining > 0 {
            let frag = self.fragments.as_mut().unwrap();
            frag.data_start += remaining;
        }

        self.length -= size;
        assert!(self.fragments.is_some() || self.length == 0);
    }

    /// Add all data in the passed slice to the end of this buffer.
    pub fn append_from_slice(&mut self, data: &[u8]) {
        if data.len() == 0 {
            return;
        }

        // Find the last frag (or, if the buffer is empty, create a new one)
        let mut last_frag = if self.fragments.is_none() {
            self.fragments = Some(alloc_fragment());
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
            let copy_len = cmp::min(FRAGMENT_SIZE - frag.data_end, data.len() - data_offset);
            frag.data[frag.data_end..frag.data_end + copy_len]
                .copy_from_slice(&data[data_offset..data_offset + copy_len]);
            frag.data_end += copy_len;
            data_offset += copy_len;
            if data_offset < data.len() {
                let new_frag = Some(alloc_fragment());
                last_frag.as_mut().unwrap().next = new_frag;
                last_frag = &mut last_frag.as_mut().unwrap().next;
            }
        }

        self.length += data.len();
    }

    /// Copy data out of the buffer into a slice, leaving the NetBuffer
    /// unmodified.
    pub fn copy_to_slice(&self, dest: &mut [u8]) -> usize {
        let mut copied = 0;
        let mut iter = self.iter(usize::MAX);
        while copied < dest.len() {
            let next = iter.next();
            if next.is_none() {
                break;
            }

            let slice = next.unwrap();
            let copy_len = cmp::min(slice.len(), dest.len() - copied);
            dest[copied..copied + copy_len].copy_from_slice(&slice[..copy_len]);
            copied += copy_len;
        }

        assert!(copied <= self.length);
        assert!(copied <= dest.len());
        assert!(copied == self.length || copied == dest.len());
        assert!(copied != self.len() || iter.next().is_none());

        return copied;
    }

    /// Copy data out of another buffer into this one, leaving the original
    /// unmodified.
    pub fn append_from_buffer(&mut self, other: &NetBuffer, length: usize) {
        for frag in other.iter(length) {
            self.append_from_slice(frag);
        }
    }

    /// This just takes over data from another buffer, tacking it onto the
    /// end. Rust's move semantics kind of shine here, because this
    /// takes over the storage with no copies.
    pub fn append_buffer(&mut self, mut other: NetBuffer) {
        self.length += other.length;
        if self.fragments.is_none() {
            self.fragments = other.fragments.take();
        } else {
            let mut last_frag = self.fragments.as_mut().unwrap();
            while last_frag.next.is_some() {
                last_frag = last_frag.next.as_mut().unwrap();
            }

            last_frag.next = other.fragments.take();
        }
    }
}

impl<'a> Iterator for BufferIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.current_frag.is_none() || self.remaining == 0 {
            return None;
        }

        let frag = self.current_frag.as_ref().unwrap();
        let slice_length = cmp::min(frag.len(), self.remaining);
        assert!(self.remaining >= slice_length);
        let start_offs = frag.data_start;
        let slice = &frag.data[start_offs..start_offs + slice_length];
        self.remaining -= slice_length;
        self.current_frag = &frag.next;

        Some(slice)
    }
}

mod tests {
    fn no_leaks() -> bool {
        unsafe { super::g_free_bufs == super::g_pool_size }
    }

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
        buf.copy_to_slice(&mut dest);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        std::mem::drop(buf);
        assert!(no_leaks());
    }


    #[test]
    fn test_copy_to_slice1() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        // Destination slice is larger than buffer
        let mut dest = [0; 20];
        let copied = buf.copy_to_slice(&mut dest);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0, 0]);
        assert_eq!(copied, 15);
        assert_eq!(buf.len(), 15);

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_copy_to_slice2() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        // Destination slice is smaller than buffer
        let mut dest = [0; 10];
        let copied = buf.copy_to_slice(&mut dest);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(copied, 10);

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_copy_empty_buffer_to_slice() {
        let buf = super::NetBuffer::new();
        let mut dest = [0; 10];
        let copied = buf.copy_to_slice(&mut dest);
        assert_eq!(copied, 0);

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_grow_buffer() {
        let mut buf = super::NetBuffer::new();

        // Fill up the first frag
        let slice1 = [1; 500];
        buf.append_from_slice(&[1; 500]);
        assert_eq!(buf.len(), 500);

        // Add another frag. This will first fill the remainder of the
        // last frag, then add several new ones.
        let slice2 = [2; 1500];
        buf.append_from_slice(&[2; 1500]);
        assert_eq!(buf.len(), 2000);

        // Check the contents
        let mut dest = [0; 2000];
        buf.copy_to_slice(&mut dest);
        assert_eq!(dest[..500], slice1);
        assert_eq!(dest[500..], slice2);

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_alloc_header() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 100]);
        assert!(buf.len() == 100);

        // This will add a new fragment
        buf.alloc_header(20);
        assert_eq!(buf.len(), 120);
        {
            let header = buf.header_mut();
            header[0] = 1;
            header[19] = 2;
        }
        buf.alloc_header(20);
        assert_eq!(buf.len(), 140);
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

        std::mem::drop(buf);
        assert!(no_leaks());
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
        buf.copy_to_slice(&mut dest);
        for i in 0..492 {
            assert_eq!(dest[i], (i + 20) as u8);
        }

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_iter1() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        buf.append_from_slice(&[2; 512]);
        buf.append_from_slice(&[3; 512]);

        // This range will chop the last frag
        let mut iter = buf.iter(1500);
        let slice1 = iter.next().unwrap();
        assert_eq!(slice1.len(), 512);
        assert_eq!(slice1[0], 1);
        assert_eq!(slice1[511], 1);
        let slice2 = iter.next().unwrap();
        assert_eq!(slice2.len(), 512);
        assert_eq!(slice2[0], 2);
        assert_eq!(slice2[511], 2);
        let slice3 = iter.next().unwrap();
        assert_eq!(slice3.len(), 476);
        assert_eq!(slice3[0], 3);
        assert_eq!(slice3[475], 3);
        assert!(iter.next().is_none());

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_iter2() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);

        // Zero length
        let mut iter = buf.iter(0);
        assert!(iter.next().is_none());

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_iter3() {
        // Create iterator on empty buffer
        let buf = super::NetBuffer::new();
        let mut iter = buf.iter(usize::MAX);
        assert!(iter.next().is_none());
        std::mem::drop(buf);
        assert!(no_leaks());
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
        buf1.copy_to_slice(&mut dest);
        assert_eq!(dest[0..512], [1; 512]);
        assert_eq!(dest[512..1024], [2; 512]);
        assert_eq!(dest[1024..1536], [3; 512]);
        assert_eq!(buf1.len(), 3072);

        std::mem::drop(buf1);
        assert!(no_leaks());

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
        buf1.copy_to_slice(&mut dest);
        assert_eq!(dest[0..512], [1; 512]);
        assert_eq!(dest[512..1024], [2; 512]);
        assert_eq!(dest[1024..1536], [3; 512]);
        assert_eq!(dest[1536..2048], [4; 512]);
        assert_eq!(dest[2048..2536], [5; 488]);

        assert_eq!(buf1.len(), 2536);

        std::mem::drop(buf1);
        std::mem::drop(buf2);
        assert!(no_leaks());
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
        buf.copy_to_slice(&mut dest);
        assert_eq!(dest[0], 0xcc);
        assert_eq!(dest[19], 0x55);
        assert_eq!(dest[20], 1);
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

        std::mem::drop(buf);
        assert!(no_leaks());
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

        std::mem::drop(buf);
        assert!(no_leaks());
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

        std::mem::drop(buf);
        assert!(no_leaks());
    }

    #[test]
    fn test_receive_flow() {
        // Run sequence of operations that happens when receiving a packet to
        // ensure there are no bad interactions between them.
        let mut buf = super::NetBuffer::new();
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        assert_eq!(buf.len(), 512);

        // Process protocol layers
        let x = buf.header();
        assert_eq!(x[5], 5);
        buf.trim_head(40);
        assert_eq!(buf.len(), 472);
        let y = buf.header();
        assert_eq!(y[0], 40);
        buf.trim_head(20);
        assert_eq!(buf.len(), 452);

        // Append
        let mut receive_queue = super::NetBuffer::new();
        receive_queue.append_buffer(buf);
        assert_eq!(receive_queue.len(), 452);

        // Read
        let mut dest = [0; 452];
        receive_queue.copy_to_slice(&mut dest);
        for i in 0..452 {
            assert_eq!(dest[i], (i + 60) as u8);
        }

        receive_queue.trim_head(452);
        assert_eq!(receive_queue.len(), 0);

        std::mem::drop(receive_queue);
        assert!(no_leaks());
    }

    #[test]
    fn test_transmit_flow() {
        // Run sequence of operations that happens when transmitting a packet to
        // ensure there are no bad interactions between them.

        // Create a packet
        let mut buf = super::NetBuffer::new();
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        assert_eq!(buf.len(), 512);

        // Process protocol layers
        buf.alloc_header(40);
        assert_eq!(buf.len(), 552);
        let x = buf.header_mut();
        x[0] = 100;
        x[39] = 102;
        buf.alloc_header(20);
        assert_eq!(buf.len(), 572);
        let y = buf.header_mut();
        y[0] = 103;
        y[19] = 104;

        // Transmit
        let mut out_data = [0; 572];
        buf.copy_to_slice(&mut out_data);
        assert_eq!(out_data[0], 103);
        assert_eq!(out_data[19], 104);
        assert_eq!(out_data[20], 100);
        assert_eq!(out_data[59], 102);
        assert_eq!(out_data[60], 0);
        assert_eq!(out_data[67], 7);
        assert_eq!(out_data[570], 254);
        assert_eq!(out_data[571], 255);

        std::mem::drop(buf);
        assert!(no_leaks());
    }
}
