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

use lazy_static::lazy_static;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TIMER_INTERVAL: Duration = Duration::from_millis(50);

struct Timer {
    absolute_timeout: u64,
    closure: Option<Box<dyn FnOnce() + Send + Sync>>,
    id: i32,
}

lazy_static! {
    static ref TIMER_LIST: Mutex<Vec<Timer>> = Mutex::new(Vec::new());
    static ref NEXT_TIMER_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
}

fn current_time_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// Valid timer IDs are always positive (this allows callers to use -1 to indicate
/// no timer is pending)
pub fn set_timer<F>(timeout_ms: u32, closure: F) -> i32
where
    F: FnOnce() + Send + Sync + 'static
{
    let mut list = TIMER_LIST.lock().unwrap();

    let id = (NEXT_TIMER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        & 0x7fffffff) as i32;
    list.push(Timer {
        absolute_timeout: current_time_ms() + timeout_ms as u64,
        closure: Some(Box::new(closure)),
        id: id,
    });

    id
}

pub fn cancel_timer(timer_id: i32) -> bool {
    let mut list = TIMER_LIST.lock().unwrap();
    for i in 0..list.len() {
        let timer = &list[i];
        if timer.id == timer_id {
            list.swap_remove(i);
            return true;
        }
    }

    false
}

pub fn init() {
    std::thread::spawn(|| {
        loop {
            sleep(TIMER_INTERVAL);
            let mut list = TIMER_LIST.lock().unwrap();
            let now = current_time_ms();
            let mut i = 0;
            while i < list.len() {
                if now >= list[i].absolute_timeout {
                    let timer = list.remove(i);
                    let closure = timer.closure;
                    drop(list); // Unlock
                    (closure.unwrap())();
                    list = TIMER_LIST.lock().unwrap();
                } else {
                    i += 1;
                }
            }
        }
    });
}


