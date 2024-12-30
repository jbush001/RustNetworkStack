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

// XXX Very hacky...

use lazy_static::lazy_static;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::thread::sleep;
use std::sync::Mutex;
use std::any::Any;

const MAX_TIMERS: usize = 32;
const TIMER_INTERVAL: Duration = Duration::from_millis(50);
pub type TimerData = Option<Box<dyn Any + Send>>;
pub type TimerCallback = fn(TimerData);

struct Timer {
    absolute_timeout: u64,
    callback: TimerCallback,
    data: TimerData,
    pending: bool,
    version: u32,
}

lazy_static! {
    static ref TIMER_LIST: Mutex<Vec<Timer>> = Mutex::new((0..MAX_TIMERS).map(|_| Timer {
        absolute_timeout: 0,
        callback: |_| {},
        data: None,
        pending: false,
        version: 0,
    }).collect());
}

fn current_time_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn generate_id(index: usize, version: u32) -> i32 {
    (((version * MAX_TIMERS as u32) & 0x7fffff00) as i32) + index as i32
}

/// Valid timer IDs are always positive (this allows callers to use -1 to indicate
/// no timer is pending)
pub fn set_timer(timeout_ms: u32, callback: TimerCallback, data: TimerData) -> i32 {
    let mut list = TIMER_LIST.lock().unwrap();
    for i in 0..MAX_TIMERS {
        let timer = &mut list[i];
        if !timer.pending {
            timer.absolute_timeout = current_time_ms() + timeout_ms as u64;
            timer.callback = callback;
            timer.data = data;
            timer.pending = true;
            timer.version += 1;
            return generate_id(i, timer.version);
        }
    }

    panic!("Out of timers");
}

pub fn cancel_timer(timer_id: i32) -> bool {
    let index = timer_id as usize % (MAX_TIMERS as usize);
    let mut list = TIMER_LIST.lock().unwrap();
    let timer = &mut list[index];
    let this_id = generate_id(index, timer.version);
    if timer_id == this_id {
        timer.pending = false;
        true
    } else {
        false
    }
}

pub fn init() {
    std::thread::spawn(|| {
        loop {
            sleep(TIMER_INTERVAL);
            let mut list = TIMER_LIST.lock().unwrap();
            let now = current_time_ms();
            for i in 0..MAX_TIMERS {
                let timer = &mut list[i];
                if timer.pending {
                    if now >= timer.absolute_timeout {
                        timer.pending = false;

                        let callback = timer.callback;
                        let data = timer.data.take();
                        drop(list);  // Unlock
                        (callback)(data);
                        list = TIMER_LIST.lock().unwrap();
                    }
                }
            }
        }
    });
}
