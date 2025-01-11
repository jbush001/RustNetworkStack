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

use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

//
// General purpose timer API.
// Timers are set and cancelled frequently, often without expiring. For example,
// whenever data is sent or received, there is usually a timer for handling
// retransmission or deferred acknowledgements. As such, this doesn't use any
// kind of sorted data structure, which would have a overhead for all of the
// unnecessary insertions and deletions (and is trickier to implement in Rust's
// ownership model, generally requiring some sort of doubly linked list).
// the tradeoff is that this must scan the list of active timers for every tick.
// Given the assumption that the total number of  timers is relatively small,
// this seems like a reasonable, but obviously would run into scaling issues
// in a real system.
//
// Alternatives:
// - A "timer wheel" is a data structure that reduces the overhead of sorted
//   insertions by hashing the timeout.
// - Various sorts of priority queues, heaps, etc.
//

use std::sync::LazyLock;

const TIMER_INTERVAL: Duration = Duration::from_millis(50);

struct Timer {
    absolute_timeout_ms: u64,
    closure: Option<Box<dyn FnOnce() + Send + Sync>>,
    id: i32,
}

static PENDING_TIMERS: LazyLock<Mutex<Vec<Timer>>> = LazyLock::new(|| {
    Mutex::new(Vec::new())
});

static NEXT_TIMER_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Returns a timer ID, which can be passed to cancel_timer to disable it.
/// Valid timer IDs are always positive (this allows callers to use -1 to indicate
/// no timer is pending).
/// The timeout is relative to the current time.
pub fn set_timer<F>(timeout_ms: u32, closure: F) -> i32
where
    F: FnOnce() + Send + Sync + 'static,
{
    let mut list = PENDING_TIMERS.lock().unwrap();

    let id = (NEXT_TIMER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0x7fffffff) as i32;
    list.push(Timer {
        absolute_timeout_ms: current_time_ms() + timeout_ms as u64,
        closure: Some(Box::new(closure)),
        id,
    });

    id
}

/// Returns true if the timer was already pending, false if had
/// already expired.
pub fn cancel_timer(timer_id: i32) -> bool {
    let mut list = PENDING_TIMERS.lock().unwrap();
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
            let mut list = PENDING_TIMERS.lock().unwrap();
            let now = current_time_ms();
            let mut i = 0;
            while i < list.len() {
                if now >= list[i].absolute_timeout_ms {
                    let timer = list.remove(i);
                    let closure = timer.closure;

                    // Dropping the list guard object will unlock the mutex.
                    // This is necessary because timer callbacks will often
                    // call back to set another timer. This would deadlock if
                    // the lock was held.
                    drop(list);
                    (closure.unwrap())();

                    // Reacquire the lock before continuing to scan the list.
                    list = PENDING_TIMERS.lock().unwrap();
                } else {
                    i += 1;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, Once};

    static START_TIMER_THREAD: Once = Once::new();

    fn start_timer_thread() {
        START_TIMER_THREAD.call_once(|| {
            init();
        });
    }

    #[test]
    fn test_set_timer() {
        start_timer_thread();

        let flag = Arc::new(Mutex::new(false));
        let flag_clone = Arc::clone(&flag);

        set_timer(100, move || {
            let mut flag = flag_clone.lock().unwrap();
            *flag = true;
        });

        sleep(Duration::from_millis(300));
        assert_eq!(*flag.lock().unwrap(), true);
    }

    #[test]
    fn test_cancel_timer() {
        start_timer_thread();

        let flag = Arc::new(Mutex::new(false));
        let flag_clone = Arc::clone(&flag);

        let timer_id = set_timer(100, move || {
            let mut flag = flag_clone.lock().unwrap();
            *flag = true;
        });

        assert_eq!(cancel_timer(timer_id), true);
        sleep(Duration::from_millis(300));
        assert_eq!(*flag.lock().unwrap(), false);
    }

    #[test]
    fn test_multiple_timers() {
        start_timer_thread();

        let flag1 = Arc::new(Mutex::new(false));
        let flag2 = Arc::new(Mutex::new(false));
        let flag1_clone = Arc::clone(&flag1);
        let flag2_clone = Arc::clone(&flag2);

        set_timer(500, move || {
            let mut flag = flag1_clone.lock().unwrap();
            *flag = true;
        });

        set_timer(100, move || {
            let mut flag = flag2_clone.lock().unwrap();
            *flag = true;
        });

        sleep(Duration::from_millis(300));
        assert_eq!(*flag1.lock().unwrap(), false);
        assert_eq!(*flag2.lock().unwrap(), true);

        sleep(Duration::from_millis(400));
        assert_eq!(*flag1.lock().unwrap(), true);
    }
}
