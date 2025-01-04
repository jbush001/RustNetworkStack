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

// Isolating buffer calls can be a bit challenging. Because these calls are
// very short duration, the benchmark calls them many times in a row, then
// divides the total time by the number of repetitions to get a more accurate
// time. However, this doens't work well for buffers, since this would allocate
// a lot of memory. So we need to pair allocations with deallocations, which
// makes it difficult to isolate individual calls.

use criterion::{criterion_group, criterion_main, Criterion};
use netstack::buf;

// A baseline of sorts that we can compare other calls to. Allocates
// a single buffer then immediately frees it.
pub fn prealloc_free(c: &mut Criterion) {
    c.bench_function("prealloc_free", |b| b.iter(|| {
        let mut buf = buf::NetBuffer::new_prealloc(512);
    }));
}

pub fn copy_to_slice_small(c: &mut Criterion) {
    let mut buf = buf::NetBuffer::new_prealloc(512);

    let mut dst = [0; 512];
    c.bench_function("copy_to_slice_small", |b| b.iter(|| {
        buf.copy_to_slice(&mut dst);
    }));
}

pub fn copy_to_slice_large(c: &mut Criterion) {
    let mut buf = buf::NetBuffer::new_prealloc(0x10000);

    let mut dst = [0; 0x10000];
    c.bench_function("copy_to_slice_large", |b| b.iter(|| {
        buf.copy_to_slice(&mut dst);
    }));
}


pub fn alloc_header_fast(c: &mut Criterion) {
    c.bench_function("alloc_header_fast", |b| b.iter(|| {
        let mut buf = buf::NetBuffer::new();
        for _ in 0..512 {
            buf.alloc_header(1);
        }
    }));
}

pub fn alloc_header_slow(c: &mut Criterion) {
    c.bench_function("alloc_header_slow", |b| b.iter(|| {
        let mut buf = buf::NetBuffer::new();
        for _ in 0..128 {
            buf.alloc_header(512);
        }
    }));
}

pub fn trim_head(c: &mut Criterion) {
    c.bench_function("trim_head", |b| b.iter(|| {
        let mut buf = buf::NetBuffer::new_prealloc(0x10000);
        for _ in 0..4096 {
            buf.trim_head(16);
        }
    }));
}

pub fn trim_tail(c: &mut Criterion) {
    c.bench_function("trim_tail", |b| b.iter(|| {
        let mut buf = buf::NetBuffer::new_prealloc(0x10000);
        for _ in 0..4096 {
            buf.trim_tail(16);
        }
    }));
}

pub fn append_from_buffer_small(c: &mut Criterion) {
    let mut buf1 = buf::NetBuffer::new_prealloc(512);

    c.bench_function("append_from_buffer_small", |b| b.iter(|| {
        let mut buf2 = buf::NetBuffer::new();
        buf2.append_from_buffer(&buf1, usize::MAX);
    }));
}

pub fn append_from_buffer_large(c: &mut Criterion) {
    let mut buf1 = buf::NetBuffer::new_prealloc(0x10000);

    c.bench_function("append_from_buffer_large", |b| b.iter(|| {
        let mut buf2 = buf::NetBuffer::new();
        buf2.append_from_buffer(&buf1, usize::MAX);
    }));
}

criterion_group!(benches,
    prealloc_free,
    copy_to_slice_small,
    copy_to_slice_large,
    alloc_header_fast,
    alloc_header_slow,
    trim_head,
    trim_tail,
    append_from_buffer_small,
    append_from_buffer_large,
);

criterion_main!(benches);
