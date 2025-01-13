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

pub mod buf;
pub mod icmp;
mod ip;
mod netif;
pub mod tcp;
mod timer;
pub mod udp;
pub mod util;

fn packet_receive_thread() {
    loop {
        let packet = netif::recv_packet();
        ip::ip_input(packet);
    }
}

pub fn init_netstack() {
    netif::init();
    timer::init();
    std::thread::spawn(|| {
        packet_receive_thread();
    });
}
