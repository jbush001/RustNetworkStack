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
use crate::ipv4;
use crate::util;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use lazy_static::lazy_static;

pub struct UDPSocket {
    receive_queue: Vec<(util::IPv4Addr, u16, buf::NetBuffer)>,
    port: u16,
}

lazy_static! {
    static ref PORT_MAP: Mutex<HashMap<u16, Arc<Mutex<UDPSocket>>>> = Mutex::new(HashMap::new());
}

impl UDPSocket {
    pub fn new(port: u16) -> Arc<Mutex<UDPSocket>> {
        let socket = UDPSocket {
            receive_queue: Vec::new(),
            port: port,
        };

        let handle = Arc::new(Mutex::new(socket));
        PORT_MAP.lock().unwrap().insert(port, handle.clone());
        handle
    }

    pub fn receive(&mut self) -> Option<(util::IPv4Addr, u16, Vec<u8>)> {
        let entry = self.receive_queue.pop();
        if entry.is_none() {
            return None;
        }

        let (source_addr, source_port, buf) = entry.unwrap();
        Some((source_addr, source_port, buf.payload().to_vec()))
    }

    pub fn send(&mut self, dest_addr: util::IPv4Addr, dest_port: u16, data: &[u8]) {
        let mut packet = buf::NetBuffer::new();
        packet.append_from_slice(data);
        udp_send(packet, dest_addr, self.port, dest_port);
    }
}


//    0               1               2               3
//    +-------------------------------+-------------------------------+
//  0 |         Source Port           |          Dest Port            |
//    +-------------------------------+-------------------------------+
//  4 |            Length             |           Checksum            |
//    +-------------------------------+-------------------------------+

const UDP_HEADER_LEN: usize = 8;

pub fn udp_recv(mut packet: buf::NetBuffer, source_addr: util::IPv4Addr) {
    println!("Got UDP packet");

    let payload = packet.payload();
    let source_port = util::get_be16(&payload[0..2]);
    let dest_port = util::get_be16(&payload[2..4]);
    let length = util::get_be16(&payload[4..6]);
    packet.remove_header(UDP_HEADER_LEN);

    println!("Source port {} dest port {}", source_port, dest_port);
    println!("Length {}", length);

    let mut port_map_guard = PORT_MAP.lock().unwrap();
    let socket = port_map_guard.get_mut(&dest_port);
    if socket.is_none() {
        println!("No socket listening on port {}", dest_port);
        return;
    }

    socket.unwrap().lock().unwrap().receive_queue.push((source_addr, source_port, packet));
}

fn udp_send(
    mut packet: buf::NetBuffer,
    dest_addr: util::IPv4Addr,
    source_port: u16,
    dest_port: u16,
) {
    packet.add_header(UDP_HEADER_LEN);
    let length = packet.payload_len() as u16;
    let payload = packet.mut_payload();
    util::set_be16(&mut payload[0..2], source_port);
    util::set_be16(&mut payload[2..4], dest_port);
    util::set_be16(&mut payload[4..6], length);
    util::set_be16(&mut payload[6..8], 0); // Skip computing checksum

    ipv4::ip_send(packet, ipv4::PROTO_UDP, dest_addr);
}
