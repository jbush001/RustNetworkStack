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
use crate::ip;
use crate::util;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Condvar;
use std::sync::{Arc, Mutex};

pub struct UDPSocket {
    receive_queue: VecDeque<(util::IPAddr, u16, buf::NetBuffer)>,
    port: u16,
}

type SocketReference = Arc<(Mutex<UDPSocket>, Condvar)>;
type PortMap = HashMap<u16, SocketReference>;


lazy_static! {
    static ref PORT_MAP: Mutex<PortMap> = Mutex::new(HashMap::new());
}

impl UDPSocket {
    fn new(port: u16) -> UDPSocket {
        UDPSocket {
            receive_queue: VecDeque::new(),
            port,
        }
    }
}

pub fn udp_open(port: u16) -> Result<SocketReference, &'static str> {
    let mut port_map_guard = PORT_MAP.lock().unwrap();
    if port_map_guard.contains_key(&port) {
        return Err("Port already in use");
    }

    let socket = UDPSocket::new(port);
    let socket = Arc::new((Mutex::new(socket), Condvar::new()));
    port_map_guard.insert(port, socket.clone());

    Ok(socket)
}

pub fn udp_recv(
    socket: &mut SocketReference,
    data: &mut [u8],
    out_addr: &mut util::IPAddr,
    out_port: &mut u16,
) -> i32 {
    let (mutex, cond) = &**socket;
    let mut guard = mutex.lock().unwrap();

    loop {
        let entry = guard.receive_queue.pop_front();
        if entry.is_some() {
            let (source_addr, source_port, buf) = entry.unwrap();
            *out_addr = source_addr;
            *out_port = source_port;
            let len = buf.len();
            let copy_len = std::cmp::min(len, data.len());
            buf.copy_to_slice(&mut data[0..copy_len]);
            return copy_len as i32;
        }

        // Need to wait for data
        guard = cond.wait(guard).unwrap();
    }
}

pub fn udp_send(
    socket: &mut SocketReference,
    dest_addr: util::IPAddr,
    dest_port: u16,
    data: &[u8],
) -> Result<(), &'static str> {
    if !matches!(dest_addr, util::IPAddr::V4(_)) {
        return Err("Only IPv4 is supported");
    }

    let (mutex, _cond) = &**socket;
    let guard = mutex.lock().unwrap();

    let mut packet = buf::NetBuffer::new();
    packet.append_from_slice(data);
    udp_output(packet, dest_addr, guard.port, dest_port);

    Ok(())
}

//    0               1               2               3
//    +-------------------------------+-------------------------------+
//  0 |         Source Port           |          Dest Port            |
//    +-------------------------------+-------------------------------+
//  4 |            Length             |           Checksum            |
//    +-------------------------------+-------------------------------+

const UDP_HEADER_LEN: usize = 8;

pub fn udp_input(mut packet: buf::NetBuffer, source_addr: util::IPAddr) {
    let header = packet.header();
    let source_port = util::get_be16(&header[0..2]);
    let dest_port = util::get_be16(&header[2..4]);
    packet.trim_head(UDP_HEADER_LEN);

    let mut port_map_guard = PORT_MAP.lock().unwrap();
    let pm_entry = port_map_guard.get_mut(&dest_port);
    if pm_entry.is_none() {
        println!("No socket listening on port {}", dest_port);
        return;
    }

    let socket = pm_entry
        .expect("just checked if pm_entry is none above")
        .clone();
    let (mutex, cond) = &*socket;
    let mut guard = mutex.lock().unwrap();

    guard
        .receive_queue
        .push_back((source_addr, source_port, packet));

    cond.notify_all();
}

fn udp_output(
    mut packet: buf::NetBuffer,
    dest_addr: util::IPAddr,
    source_port: u16,
    dest_port: u16,
) {
    packet.alloc_header(UDP_HEADER_LEN);
    let length = packet.len() as u16;
    let header = packet.header_mut();
    util::set_be16(&mut header[0..2], source_port);
    util::set_be16(&mut header[2..4], dest_port);
    util::set_be16(&mut header[4..6], length);
    util::set_be16(&mut header[6..8], 0); // Skip computing checksum

    ip::ip_output(packet, ip::PROTO_UDP, dest_addr);
}
