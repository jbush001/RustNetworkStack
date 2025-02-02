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

// User Datagram Protcol, as described in RFC 768

use crate::buf;
use crate::ip;
use crate::netif;
use crate::util;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Condvar;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

pub type SocketReference = Arc<UDPSocket>;

pub struct UDPSocket(Mutex<UDPSocketState>, Condvar);

pub struct UDPSocketState {
    receive_queue: VecDeque<(util::IPAddr, u16, buf::NetBuffer)>,
    port: u16,
}

type PortMap = HashMap<u16, SocketReference>;

static PORT_MAP: LazyLock<Mutex<PortMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

impl UDPSocket {
    fn new(port: u16) -> UDPSocket {
        UDPSocket(Mutex::new(UDPSocketState::new(port)), Condvar::new())
    }

    fn lock(&self) -> (MutexGuard<UDPSocketState>, &Condvar) {
        (self.0.lock().unwrap(), &self.1)
    }
}

impl UDPSocketState {
    fn new(port: u16) -> UDPSocketState {
        UDPSocketState {
            receive_queue: VecDeque::new(),
            port,
        }
    }
}

/// Open a new UDP socket with the specified local port.
pub fn udp_open(port: u16) -> Result<SocketReference, &'static str> {
    let mut port_map_guard = PORT_MAP.lock().unwrap();
    if port_map_guard.contains_key(&port) {
        return Err("Port already in use");
    }

    let socket_ref = Arc::new(UDPSocket::new(port));
    port_map_guard.insert(port, socket_ref.clone());

    Ok(socket_ref)
}

/// Wait for a UDP packet to arrive on the specified socket, copy its payload
/// into the passed slice and return the number of bytes copied.
pub fn udp_recv(
    socket_ref: &mut SocketReference,
    data: &mut [u8],
    out_addr: &mut util::IPAddr,
    out_port: &mut u16,
) -> i32 {
    let (mut guard, cond) = (*socket_ref).lock();

    loop {
        let entry = guard.receive_queue.pop_front();
        if entry.is_some() {
            let (source_addr, source_port, buf) = entry.unwrap();
            *out_addr = source_addr;
            *out_port = source_port;
            let len = buf.len();
            let copy_len = std::cmp::min(len, data.len());
            buf.copy_to_slice(&mut data[..copy_len]);
            return copy_len as i32;
        }

        // Need to wait for data
        guard = cond.wait(guard).unwrap();
    }
}

/// Send a UDP packet to the specified destination address and port.
pub fn udp_send(
    socket_ref: &mut SocketReference,
    dest_addr: util::IPAddr,
    dest_port: u16,
    data: &[u8],
) -> Result<(), &'static str> {
    let (guard, _) = (*socket_ref).lock();

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

/// Called by IP layer to handle received packets.
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
    let (mut guard, cond) = (*socket).lock();
    guard
        .receive_queue
        .push_back((source_addr, source_port, packet));

    cond.notify_all();
}

fn udp_output(mut packet: buf::NetBuffer, dest_ip: util::IPAddr, source_port: u16, dest_port: u16) {
    packet.alloc_header(UDP_HEADER_LEN);
    let length = packet.len() as u16;
    let header = packet.header_mut();
    util::set_be16(&mut header[0..2], source_port);
    util::set_be16(&mut header[2..4], dest_port);
    util::set_be16(&mut header[4..6], length);

    let ph_checksum = util::compute_pseudo_header_checksum(
        if matches!(dest_ip, util::IPAddr::V4(_)) {
            netif::get_ipaddr().0
        } else {
            netif::get_ipaddr().1
        },
        dest_ip,
        length as usize,
        ip::PROTO_UDP,
    );
    let checksum = util::compute_buffer_ones_comp(ph_checksum, &packet) ^ 0xffff;

    let header = packet.header_mut();
    util::set_be16(&mut header[6..8], checksum);
    ip::ip_output(packet, ip::PROTO_UDP, dest_ip);
}
