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

const MAX_TIMERS: usize = 32;
const TIMER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Copy, Clone)]
struct Timer {
    absolute_timeout: u64,
    callback: fn(),
    pending: bool,
    version: u32,
}

lazy_static! {
    static ref TIMER_LIST: Mutex<[Timer; MAX_TIMERS]> = Mutex::new([Timer {
        absolute_timeout: 0,
        callback: || {},
        pending: false,
        version: 0,
    }; MAX_TIMERS]);
}

fn current_time_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

pub fn set_timer(timeout_ms: u32, callback: fn()) -> u32 {
    let mut list = TIMER_LIST.lock().unwrap();
    for i in 0..MAX_TIMERS {
        let timer = &mut list[i];
        if !timer.pending {
            timer.absolute_timeout = current_time_ms() + timeout_ms as u64;
            timer.callback = callback;
            timer.pending = true;
            timer.version += 1;
            return timer.version * MAX_TIMERS as u32 + i as u32;
        }
    }

    panic!("Out of timers");
}

pub fn cancel_timer(timer_id: u32) -> bool {
    let index = timer_id as usize % (MAX_TIMERS as usize);
    let version = timer_id / (MAX_TIMERS as u32);
    let mut list = TIMER_LIST.lock().unwrap();
    let timer = &mut list[index];
    if timer.version == version {
        timer.pending = false;
        true
    } else {
        false
    }
}

pub fn init() {
    std::thread::spawn(move || {
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
                        drop(list);
                        callback();
                        list = TIMER_LIST.lock().unwrap();
                    }
                }
            }
        }
    });
}
