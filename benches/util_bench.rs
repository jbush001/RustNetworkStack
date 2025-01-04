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

use netstack::util;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

pub fn compute_ones_comp(c: &mut Criterion) {
    let buf = [0; 512];
    c.bench_function("compute_ones_comp", |b| b.iter(|| {
        black_box(util::compute_ones_comp(0, &buf));
    }));
}

pub fn compute_buffer_ones_comp_small(c: &mut Criterion) {
    let buf = [0xff; 512];
    c.bench_function("compute_buffer_ones_comp_small", |b| b.iter(|| {
        black_box(util::compute_ones_comp(0, &buf));
    }));
}

pub fn compute_buffer_ones_comp_large(c: &mut Criterion) {
    let buf = [0xff; 512];
    c.bench_function("compute_buffer_ones_comp_large", |b| b.iter(|| {
        black_box(util::compute_ones_comp(0, &buf));
    }));
}

pub fn set_be16(c: &mut Criterion) {
    let mut buf = [0; 512];
    c.bench_function("set_be16", |b| b.iter(|| {
        util::set_be16(&mut buf, 0x1234);
    }));
}

pub fn get_be32(c: &mut Criterion) {
    let buf = [0; 512];
    c.bench_function("get_be32", |b| b.iter(|| {
        black_box(util::get_be32(&buf));
    }));
}

criterion_group!(benches,
    compute_ones_comp,
    compute_buffer_ones_comp_small,
    compute_buffer_ones_comp_large,
    set_be16,
    get_be32
);

criterion_main!(benches);
