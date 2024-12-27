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

use crate::buf;
use crate::util;

//
//    0               1               2               3
//    +-------------------------------+-------------------------------+
//  0 |         Source Port           |          Dest Port            |
//    +-------------------------------+-------------------------------+
//  4 |                        Sequence Number                        |
//    +-------------------------------+-------------------------------+
//  8 |                           Ack Number                          |
//    +--------+-------------+--------+-------------------------------+
// 12 |  Offs  |  Reserved   |CEUAPRSF|            Window             |
//    +--------+-------------+--------+-------------------------------+
// 16 |         Checksum              |        Urgent Pointer         |
//    +-------------------------------+-------------------------------+
// 20 |                           [Options]                           |
//    +---------------------------------------------------------------+
//

pub fn tcp_input(packet: buf::NetBuffer, _source_ip: util::IPv4Addr) {
    println!("Got TCP packet");

    let payload = packet.payload();
    let source_port = util::get_be16(&payload[0..2]);
    let dest_port = util::get_be16(&payload[2..4]);
    let seq_num = util::get_be32(&payload[4..8]);
    let ack_num = util::get_be32(&payload[8..12]);
    let window = util::get_be16(&payload[14..16]);
    let fin = payload[13] & 1;
    let syn = (payload[13] >> 1) & 1;
    let rst = (payload[13] >> 2) & 1;
    let psh = (payload[13] >> 3) & 1;
    let ack = (payload[13] >> 4) & 1;

    println!("source port {} dest port {}", source_port, dest_port);
    println!("sequence {} ack {}", seq_num, ack_num);
    println!("window {}", window);
    println!(
        "Flags {}{}{}{}{}",
        if ack != 0 { "A" } else { "-" },
        if psh != 0 { "P" } else { "-" },
        if rst != 0 { "R" } else { "-" },
        if syn != 0 { "S" } else { "-" },
        if fin != 0 { "F" } else { "-" }
    );
}
