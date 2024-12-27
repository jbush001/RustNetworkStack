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
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Condvar;
use std::sync::{Arc, Mutex};

pub struct UDPSocket {
    receive_queue: Vec<(util::IPv4Addr, u16, buf::NetBuffer)>,
    port: u16,
}

lazy_static! {
    static ref PORT_MAP: Mutex<HashMap<u16, Arc<Mutex<UDPSocket>>>> = Mutex::new(HashMap::new());

    // This is not ideal, as it wakes up all threads waiting for data any time there
    // is actitiy on any socket. But we get into all kinds of reference/ownership
    // complexity if we try to associate a condition with each socket.
    static ref RECV_WAIT: Condvar = Condvar::new();
}

impl UDPSocket {
    fn new(port: u16) -> Arc<Mutex<UDPSocket>> {
        let socket = UDPSocket {
            receive_queue: Vec::new(),
            port: port,
        };

        let handle = Arc::new(Mutex::new(socket));
        PORT_MAP.lock().unwrap().insert(port, handle.clone());
        handle
    }
}

pub fn udp_open(port: u16) -> Arc<Mutex<UDPSocket>> {
    return UDPSocket::new(port);
}

pub fn udp_recv(socket: &mut Arc<Mutex<UDPSocket>>) -> (util::IPv4Addr, u16, Vec<u8>) {
    let mut guard = socket.lock().unwrap();
    loop {
        let entry = guard.receive_queue.pop();
        if !entry.is_none() {
            let (source_addr, source_port, buf) = entry.unwrap();
            return (source_addr, source_port, buf.payload().to_vec());
        }

        // Need to wait for more data
        guard = RECV_WAIT.wait(guard).unwrap();
    }
}

pub fn udp_send(
    socket: &mut Arc<Mutex<UDPSocket>>,
    dest_addr: util::IPv4Addr,
    dest_port: u16,
    data: &[u8],
) {
    let guard = socket.lock().unwrap();
    let mut packet = buf::NetBuffer::new();
    packet.append_from_slice(data);
    udp_output(packet, dest_addr, guard.port, dest_port);
}

//    0               1               2               3
//    +-------------------------------+-------------------------------+
//  0 |         Source Port           |          Dest Port            |
//    +-------------------------------+-------------------------------+
//  4 |            Length             |           Checksum            |
//    +-------------------------------+-------------------------------+

const UDP_HEADER_LEN: usize = 8;

pub fn udp_input(mut packet: buf::NetBuffer, source_addr: util::IPv4Addr) {
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

    socket
        .unwrap()
        .lock()
        .unwrap()
        .receive_queue
        .push((source_addr, source_port, packet));
    RECV_WAIT.notify_all();
}

fn udp_output(
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

    ipv4::ip_output(packet, ipv4::PROTO_UDP, dest_addr);
}
