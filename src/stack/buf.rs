//
// Copyright 2024-2025 Jeff Bush
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

use crate::util;
use std::cmp;
use std::ops::Range;
use std::sync::{LazyLock, Mutex};

//
// This module implements an efficient, flexible container for unstructured
// data, which is used as temporary storage throughout the network stack,
// including for packets and queued receive and transmit data. It is similar
// to mbufs in the BSD network stack or skbuffs in Linux, although
// this is implemented to be more idiomatic with Rust's ownership model.
//
// The design goals of any network buffering system are to minimize copies,
// avoid external heap fragmentation, and optimize allocation speed. The
// base storage unit is a fixed-size BufferFragment. These fragments are
// chained together to allow buffers to grow to arbitrary sizes. Fragments
// are allocated from a memory pool of fixed sized chunks, which is fast.
//
// Alternatives:
// - I also considered having NetBuffer contain an array of pointers
//   to fragments, which would be faster for some use cases and simpler,
//   However, this would limit the size of buffers (in lieu of reallocating
//   the array dynamically, which has its own issues).
// - Another design choice would be to use raw pointers to the fragments rather
//   than a Box. This would allow the NetBuffer to have a pointer to the tail
//   fragment, which would speed up appends, but the performance improvement
//   might be minimal and doesn't seem to justify giving up Rust safety
//   guarantees.
// - Making the fragments be ref counted would allow zero-copy sharing (for
//   example, when copying into the retransmit buffer), but it would likely
//   require fragments to be immutable, which would lead to more internal
//   fragmentation when adding headers, for example.
//

type FragPointer = Option<Box<BufferFragment>>;

/// A NetBuffer is the primary buffer abstraction. It is a variable sized
/// container for octets of data that can be grown or shrunk arbitrarily
/// without copies. NetBuffers are mutable.
pub struct NetBuffer {
    fragments: FragPointer, // Head of linked list of fragments

    // This is always equal the sum of the lengths of fragments.
    // (end - start for each). I maintain this separately to
    // speed up calls to get the length.
    length: usize,
}

const FRAGMENT_SIZE: usize = 512;

/// Portion of a buffer, which is a node in a linked list.
struct BufferFragment {
    // It seems a bit wasteful to store start and end as 64-bit integers,
    // But keeping these consistent avoids a lot of typecasting in other
    // parts (since most other building slice functions use usize)
    // I tried using smaller storage sizes for these and it had no
    // measurable performance impact.
    next: FragPointer,   // Next fragment in linked list.
    range: Range<usize>, // Start and end of valid data in this fragment.
    data: [u8; FRAGMENT_SIZE],
}

pub struct BufferIterator<'a> {
    current_frag: &'a FragPointer,
    remaining: usize, // How many more bytes to copy.
}

/// This is where fragments are allocated from (and return to). Free fragments
/// are stored in a single linked list, which makes allocation and deallocation
/// fast.
struct FragmentPool {
    free_list: FragPointer,
}

const POOL_GROW_SIZE: usize = 16;

// This is a global singleton used by everything.
static FRAGMENT_POOL: LazyLock<Mutex<FragmentPool>> =
    LazyLock::new(|| Mutex::new(FragmentPool::new()));

pub fn buffer_count_to_memory(count: u32) -> u32 {
    count * FRAGMENT_SIZE as u32
}

// Note that this instance is protected by an external mutex, so none of these
// functions are reentrant.
impl FragmentPool {
    const fn new() -> FragmentPool {
        FragmentPool { free_list: None }
    }

    // Add new nodes to fragment pool. These are individually heap allocated.
    fn grow(&mut self) {
        // When short, we stuff multiple frags into the pool. I don't know
        // that there's a super strong argument for doing this in bulk (vs.
        // one at a time) other than doing it this way is maybe less likely
        // to cause heap fragmentation (vs. intermingled with other allocations).
        // Ideally we'd allocate one big chunk and slice it up, but that is
        // at toods with Rust's ownership model.
        for _ in 0..POOL_GROW_SIZE {
            let mut frag = Box::new(BufferFragment::new());
            frag.next = self.free_list.take();
            self.free_list.replace(frag);
        }

        util::METRICS.buffers_created.add(POOL_GROW_SIZE as u32);
    }

    /// Allocate a new fragment from the pool.
    fn alloc(&mut self) -> Box<BufferFragment> {
        if self.free_list.is_none() {
            self.grow();
        }

        util::METRICS.buffers_allocated.inc();

        let mut new_frag = self.free_list.take().unwrap();
        if new_frag.next.is_some() {
            self.free_list.replace(new_frag.next.take().unwrap());
        }

        new_frag.range = 0..0;
        new_frag
    }

    /// Put a fragment back into the pool.
    /// It's still necessary to call this to explicitly return these (vs
    /// having them automatically return when they go out of scope). The
    /// Box class does have an allocator parameter, but it is marked as
    /// unstable and not fully supported.
    /// Note also that we never return fragments to the system allocator.
    fn free(&mut self, mut fragment: Box<BufferFragment>) {
        util::METRICS.buffers_freed.inc();
        fragment.next = self.free_list.take();
        self.free_list.replace(fragment);
    }
}

impl BufferFragment {
    const fn new() -> BufferFragment {
        BufferFragment {
            data: [0; FRAGMENT_SIZE],
            range: 0..0,
            next: None,
        }
    }

    fn len(&self) -> usize {
        self.range.len()
    }
}

impl Drop for BufferFragment {
    /// Fragments should always to back into the pool, and thus this hould
    /// never be called. If it is, it means ownership has inadvertently been
    /// lost (leaked)
    fn drop(&mut self) {
        panic!("BufferFragment should never be dropped");
    }
}

impl Drop for NetBuffer {
    /// When a NetBuffer goes out of scope, all of its data will be returned to
    /// the allocator pool.
    fn drop(&mut self) {
        let mut frag = self.fragments.take();
        let mut guard = FRAGMENT_POOL.lock().unwrap();
        while frag.is_some() {
            let next = frag.as_mut().unwrap().next.take();
            guard.free(frag.unwrap());
            frag = next;
        }
    }
}

impl Default for NetBuffer {
    /// Returns an empty buffer.
    fn default() -> NetBuffer {
        NetBuffer::new()
    }
}

impl NetBuffer {
    /// Create a new NetBuffer that has no data in it.
    pub const fn new() -> NetBuffer {
        NetBuffer {
            fragments: None,
            length: 0,
        }
    }

    /// Create a buffer of a specific length that is zero filled.
    /// This function is used by the underlying interface during packet
    /// reception and isn't really useful for much else.
    pub fn new_prealloc(length: usize) -> NetBuffer {
        let mut buf = NetBuffer {
            fragments: None,
            length,
        };

        let mut to_add = length;
        let mut guard = FRAGMENT_POOL.lock().unwrap();
        while to_add > 0 {
            let mut new_frag = guard.alloc();
            let frag_size = cmp::min(to_add, FRAGMENT_SIZE);
            new_frag.range = 0..frag_size;
            to_add -= frag_size;
            new_frag.next = buf.fragments.take();
            buf.fragments = Some(new_frag);
        }

        buf
    }

    /// Return the total number of octets contained within this buffer
    pub fn len(&self) -> usize {
        self.length
    }

    /// Returns true if this contains no data.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Return an iterator that will return slices that represent portions
    /// of the data in this buffer.
    pub fn iter(&self, length: usize) -> BufferIterator {
        BufferIterator {
            current_frag: &self.fragments,
            remaining: length,
        }
    }

    /// Return a slice pointing to data in the beginning of the buffer.
    /// This is used for reading header contents. Note: this slice may be larger
    /// than the size passed to add_header.
    pub fn header(&self) -> &[u8] {
        assert!(
            self.fragments.is_some(),
            "Shouldn't call header on empty buffer"
        );
        let head_frag = self.fragments.as_ref().unwrap();
        &head_frag.data[head_frag.range.clone()]
    }

    /// Same as header, but mutable. Used for writing the header.
    pub fn header_mut(&mut self) -> &mut [u8] {
        assert!(
            self.fragments.is_some(),
            "Shouldn't call header on empty buffer"
        );
        let head_frag = self.fragments.as_mut().unwrap();
        &mut head_frag.data[head_frag.range.clone()]
    }

    /// Reserve space for another header to be prepended to the buffer
    /// This method guarantees the header is always contiguous (i.e. does not
    /// span multiple fragments). The contents of the allocated space will be
    /// zeroed out.
    ///
    /// The network stack calls the protocol modules from highest to lowest
    /// layer, so this is called by each one as a packet is being prepared
    /// to send.
    pub fn alloc_header(&mut self, size: usize) {
        assert!(
            size <= FRAGMENT_SIZE,
            "Header can't be larger than a fragment"
        );
        if self.fragments.is_none() || self.fragments.as_ref().unwrap().range.start < size {
            // Prepend a new frag. We place the data at the end of the frag
            // to allow space for subsequent headers to be added.
            let mut new_head_frag = FRAGMENT_POOL.lock().unwrap().alloc();
            new_head_frag.range = FRAGMENT_SIZE - size..FRAGMENT_SIZE;
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
            frag.range.start -= size;
        }

        // Zero out contents of header.
        let frag = self.fragments.as_mut().unwrap();
        frag.data[frag.range.start..frag.range.start + size].fill(0);

        self.length += size;
    }

    /// Remove the passed number of octets from the beginning of buffer.
    pub fn trim_head(&mut self, size: usize) {
        // This condition suggests a logic error somewhere else in the
        // code, thus better to just assert than silently ignore.
        assert!(
            size <= self.len(),
            "Should not trim more than buffer length"
        );

        let mut remaining = size;

        // Remove entire buffers if needed
        let mut guard = FRAGMENT_POOL.lock().unwrap();
        while remaining > 0 {
            let frag_len = self.fragments.as_ref().unwrap().len();
            if frag_len > remaining {
                break;
            }

            remaining -= frag_len;
            let mut dead_frag = self.fragments.take().unwrap();
            self.fragments = dead_frag.next.take();
            guard.free(dead_frag);
        }

        // Truncate the first buffer
        if remaining > 0 {
            let frag = self.fragments.as_mut().unwrap();
            frag.range.start += remaining;
        }

        self.length -= size;
        assert!(
            self.fragments.is_some() || self.length == 0,
            "Should not have fragments in an empty buffer"
        );
    }

    /// Return the passed number of octets from the end of the buffer.
    pub fn trim_tail(&mut self, size: usize) {
        // This generally suggests a logic error somewhere else in the
        // code, thus better to just assert than silently ignore.
        assert!(
            size <= self.len(),
            "Should not trim more than buffer length"
        );

        if size == self.len() {
            // Remove all data
            let mut guard = FRAGMENT_POOL.lock().unwrap();
            while let Some(mut dead_frag) = self.fragments.take() {
                self.fragments = dead_frag.next.take();
                guard.free(dead_frag);
            }

            self.length = 0;
            return;
        }

        self.length -= size;
        let mut remaining = self.length;

        // Skip entire fragments that we will keep
        let mut last_frag = &mut self.fragments;
        loop {
            let length = last_frag.as_ref().unwrap().len();
            if length >= remaining {
                break;
            }

            remaining -= length;
            last_frag = &mut last_frag.as_mut().unwrap().next;
        }

        // Truncate the partial fragment
        let partial = last_frag.as_mut().unwrap();
        if partial.len() > remaining {
            partial.range.end = partial.range.start + remaining;
        }

        // Free any fragments that come after last_frag
        let mut frag = last_frag.as_mut().unwrap().next.take();
        let mut guard = FRAGMENT_POOL.lock().unwrap();
        while frag.is_some() {
            let next = frag.as_mut().unwrap().next.take();
            guard.free(frag.unwrap());
            frag = next;
        }
    }

    /// Allocate space in the end of the buffer and copy data from the passed slice
    /// to it.
    pub fn append_from_slice(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        // Find the last frag (or, if the buffer is empty, create a new one)
        let mut guard = FRAGMENT_POOL.lock().unwrap();
        let mut last_frag = if self.fragments.is_none() {
            self.fragments = Some(guard.alloc());
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
            let copy_len = cmp::min(FRAGMENT_SIZE - frag.range.end, data.len() - data_offset);
            frag.data[frag.range.end..frag.range.end + copy_len]
                .copy_from_slice(&data[data_offset..data_offset + copy_len]);
            frag.range.end += copy_len;
            data_offset += copy_len;
            if data_offset < data.len() {
                let new_frag = Some(guard.alloc());
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
        let total_to_copy = cmp::min(dest.len(), self.length);
        while copied < total_to_copy {
            let slice = iter.next().unwrap();
            let copy_len = cmp::min(slice.len(), total_to_copy - copied);
            dest[copied..(copied + copy_len)].copy_from_slice(&slice[..copy_len]);
            copied += copy_len;
        }

        assert!(copied == self.length || copied == dest.len());
        assert!(copied != self.len() || iter.next().is_none());

        copied
    }

    /// Copy data out of another buffer into this one, leaving the original
    /// unmodified.
    pub fn append_from_buffer(&mut self, other: &NetBuffer, length: usize) {
        for frag in other.iter(length) {
            self.append_from_slice(frag);
        }
    }

    /// This just takes over data from another buffer, tacking it onto the
    /// end.
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
        assert!(
            self.remaining >= slice_length,
            "Should not copy more than remaining"
        );
        let start_offs = frag.range.start;
        let slice = &frag.data[start_offs..start_offs + slice_length];
        self.remaining -= slice_length;
        self.current_frag = &frag.next;

        Some(slice)
    }
}

#[cfg(test)]
mod tests {
    // At one point, I would check at the end of each of these tests if all
    // buffers were freed, but that would fail intermittently. It turns out
    // Rust runs unit tests in parallel.

    // Walk through the buffer to ensure it is correctly formed.
    fn validate_buffer(buf: &super::NetBuffer) {
        let mut ptr = &buf.fragments;
        let mut actual_length = 0;
        while ptr.is_some() {
            let frag = ptr.as_ref().unwrap();
            // Should be non-empty and these shouldn't cross
            assert!(frag.range.start < frag.range.end, "Invalid fragment range");
            assert!(
                frag.range.end <= super::FRAGMENT_SIZE,
                "Fragment range too large"
            );
            actual_length += frag.range.end - frag.range.start;
            ptr = &frag.next;
        }

        assert_eq!(actual_length, buf.len());
    }

    #[test]
    fn test_new_prealloc() {
        let buf = super::NetBuffer::new_prealloc(1000);
        assert_eq!(buf.len(), 1000);
        assert!(!buf.is_empty());
        validate_buffer(&buf);
    }

    #[test]
    fn test_new_prealloc_zero() {
        // Doesn't make a lot of sense, but ensure it doesn't do anything weird.
        let buf = super::NetBuffer::new_prealloc(0);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        validate_buffer(&buf);
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
    }

    #[test]
    fn test_iter2() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);

        // Zero length
        let mut iter = buf.iter(0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iter_empty() {
        // Create iterator on empty buffer
        let buf = super::NetBuffer::new();
        let mut iter = buf.iter(usize::MAX);
        assert!(iter.next().is_none());
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

    #[should_panic]
    #[test]
    fn test_header_empty() {
        let buf = super::NetBuffer::new();
        let _header = buf.header();
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

    #[should_panic]
    #[test]
    fn test_header_mut_empty() {
        let mut buf = super::NetBuffer::new();
        let _header = buf.header_mut();
    }

    #[test]
    fn test_alloc_header() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 100]);
        assert!(buf.len() == 100);
        validate_buffer(&buf);

        // This will add a new fragment
        buf.alloc_header(20);
        assert_eq!(buf.len(), 120);
        validate_buffer(&buf);
        {
            let header = buf.header_mut();
            header[0] = 1;
            header[19] = 2;
        }

        // This will extend the first fragment
        buf.alloc_header(20);
        assert_eq!(buf.len(), 140);
        validate_buffer(&buf);
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
    fn test_alloc_header_empty() {
        let mut buf = super::NetBuffer::new();
        assert!(buf.is_empty());
        buf.alloc_header(20);
        assert!(!buf.is_empty());
        assert_eq!(buf.len(), 20);
        assert!(!buf.is_empty());
        validate_buffer(&buf);
    }

    #[should_panic]
    #[test]
    fn test_alloc_header_too_large() {
        // Will fail because it doesn't fit in a fragment (and it guarantees header
        // will not be split).
        let mut buf = super::NetBuffer::new();
        buf.alloc_header(1000);
    }

    #[test]
    fn test_trim_head1() {
        // Truncate the first fragment.
        let mut buf = super::NetBuffer::new();
        // Create a fragment with an incrementing count
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        assert_eq!(buf.len(), 512);
        buf.trim_head(20);
        assert!(buf.len() == 492);
        validate_buffer(&buf);
        assert!(!buf.is_empty());

        // Check the contents
        let mut dest = [0; 512];
        buf.copy_to_slice(&mut dest);
        for i in 0..492 {
            assert_eq!(dest[i], (i + 20) as u8);
        }
    }

    #[test]
    fn test_trim_head2() {
        // Remove an entire fragment and truncate part of the
        // next
        let mut buf = super::NetBuffer::new();
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        assert_eq!(buf.len(), 512);
        buf.alloc_header(20); // Prepends new fragment
        buf.trim_head(40); // Remove first fragment and then some
        assert_eq!(buf.len(), 492);
        assert!(!buf.is_empty());
        validate_buffer(&buf);
        let header = buf.header();
        assert_eq!(header[0], 20);
        assert_eq!(header[5], 25);
    }

    #[test]
    fn test_trim_head_entire_buffer() {
        // Trim removes all data in buffer.
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.trim_head(5);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        validate_buffer(&buf);
    }

    #[test]
    #[should_panic]
    fn test_trim_head_larger() {
        // The trim size is larger than the whole buffer
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.trim_head(6);
    }

    #[test]
    fn test_trim_head_zero() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.trim_head(0);
        assert_eq!(buf.len(), 5);
        assert!(!buf.is_empty());
        validate_buffer(&buf);
    }

    #[test]
    fn test_trim_tail() {
        // Truncate the last slice.
        let mut buf = super::NetBuffer::new();
        // Create a slice with an incrementing count
        let mut data = [0; 512];
        for i in 0..512 {
            data[i] = i as u8;
        }

        buf.append_from_slice(&data);
        assert!(buf.len() == 512);
        buf.trim_tail(20);
        assert!(buf.len() == 492);
        validate_buffer(&buf);

        // Check the contents
        let mut dest = [0; 512];
        buf.copy_to_slice(&mut dest);
        for i in 0..492 {
            assert_eq!(dest[i], i as u8);
        }
    }

    #[test]
    fn test_trim_tail2() {
        // Remove an entire fragment and truncate part of the
        // former.
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1; 512]);
        assert_eq!(buf.len(), 512);
        buf.append_from_slice(&[2; 20]);
        assert_eq!(buf.len(), 532);
        buf.trim_tail(40); // Remove part of the last fragment
        assert_eq!(buf.len(), 492);
        validate_buffer(&buf);

        let mut dest = [0; 492];
        buf.copy_to_slice(&mut dest);
        assert_eq!(dest[..492], [1; 492]);
    }

    #[test]
    fn test_trim_entire_buffer() {
        // Trim removes all data in buffer.
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.trim_tail(5);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        validate_buffer(&buf);
    }

    #[test]
    #[should_panic]
    fn test_trim_tail_larger() {
        // The trim size is larger than the whole buffer
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.trim_tail(6);
    }

    #[test]
    fn test_trim_tail_zero() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.trim_tail(0);
        assert_eq!(buf.len(), 5);
        validate_buffer(&buf);
    }

    #[test]
    fn test_append_from_slice() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);
        validate_buffer(&buf);
        buf.append_from_slice(&[6, 7, 8, 9, 10]);
        assert_eq!(buf.len(), 10);
        validate_buffer(&buf);
        buf.append_from_slice(&[11, 12, 13, 14, 15]);
        assert_eq!(buf.len(), 15);
        validate_buffer(&buf);

        // Check contents
        let mut dest = [0; 15];
        buf.copy_to_slice(&mut dest);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    }

    #[test]
    fn test_append_from_slice_zero() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4]);
        assert_eq!(buf.len(), 4);
        buf.append_from_slice(&[]);
        assert_eq!(buf.len(), 4);
        validate_buffer(&buf);
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
        validate_buffer(&buf);

        // Check the contents
        let mut dest = [0; 2000];
        buf.copy_to_slice(&mut dest);
        assert_eq!(dest[..500], slice1);
        assert_eq!(dest[500..], slice2);
    }

    #[test]
    fn test_grow_pool() {
        // Grow the underlying pool multiple times, then return it.
        // (regression test for an issue in original implementation)
        let mut buflist = Vec::new();
        for _ in 0..32 {
            let mut buffer = super::NetBuffer::new();
            buffer.append_from_slice(&[1; 512]);
            buflist.push(buffer);
        }
    }

    #[test]
    fn test_copy_to_slice1() {
        let mut buf = super::NetBuffer::new();
        buf.append_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        // Destination slice is larger than buffer
        let mut dest = [0; 20];
        let copied = buf.copy_to_slice(&mut dest);
        assert_eq!(
            dest,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 0, 0, 0, 0]
        );
        assert_eq!(copied, 15);
        assert_eq!(buf.len(), 15);
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
    }

    #[test]
    fn test_copy_empty_buffer_to_slice() {
        let buf = super::NetBuffer::new();
        let mut dest = [0; 10];
        let copied = buf.copy_to_slice(&mut dest);
        assert_eq!(copied, 0);
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
        validate_buffer(&buf1);
        validate_buffer(&buf2);

        // Check contents
        let mut dest = [0; 3000];
        buf1.copy_to_slice(&mut dest);
        assert_eq!(dest[0..512], [1; 512]);
        assert_eq!(dest[512..1024], [2; 512]);
        assert_eq!(dest[1024..1536], [3; 512]);
        assert_eq!(dest[1536..2048], [4; 512]);
        assert_eq!(dest[2048..2536], [5; 488]);

        assert_eq!(buf1.len(), 2536);
    }

    #[test]
    fn test_append_buffer1() {
        // Buffer being appended to is non empty
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
        validate_buffer(&buf1);

        // Check contents
        let mut dest = [0; 1536];
        buf1.copy_to_slice(&mut dest);
        assert_eq!(dest[..512], [1; 512]);
        assert_eq!(dest[512..1024], [2; 512]);
        assert_eq!(dest[1024..1536], [3; 512]);
        assert_eq!(buf1.len(), 3072);
    }

    #[test]
    fn test_append_to_empty() {
        // Buffer being appended to is empty
        let mut buf1 = super::NetBuffer::new();
        let mut buf2 = super::NetBuffer::new();
        buf2.append_from_slice(&[1; 512]);
        buf1.append_buffer(buf2);
        assert!(buf1.len() == 512);
        validate_buffer(&buf1);

        let mut dest = [0; 512];
        buf1.copy_to_slice(&mut dest);
        assert_eq!(dest[..512], [1; 512]);
    }

    #[test]
    fn test_append_empty_buffer() {
        // Buffer being appended is empty
        let mut buf1 = super::NetBuffer::new();
        buf1.append_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(buf1.len(), 5);

        let buf2 = super::NetBuffer::new();
        buf1.append_buffer(buf2);
        assert_eq!(buf1.len(), 5);
        validate_buffer(&buf1);

        let mut dest = [0; 5];
        buf1.copy_to_slice(&mut dest);
        assert_eq!(dest, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_append_empty_to_empty() {
        let mut buf1 = super::NetBuffer::new();
        let buf2 = super::NetBuffer::new();
        buf1.append_buffer(buf2);
        assert_eq!(buf1.len(), 0);
        validate_buffer(&buf1);
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
        validate_buffer(&buf);

        // Append
        let mut receive_queue = super::NetBuffer::new();
        receive_queue.append_buffer(buf);
        assert_eq!(receive_queue.len(), 452);
        validate_buffer(&receive_queue);

        // Read
        let mut dest = [0; 452];
        receive_queue.copy_to_slice(&mut dest);
        for i in 0..452 {
            assert_eq!(dest[i], (i + 60) as u8);
        }

        receive_queue.trim_head(452);
        assert_eq!(receive_queue.len(), 0);
        validate_buffer(&receive_queue);
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

        validate_buffer(&buf);

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
    }
}
