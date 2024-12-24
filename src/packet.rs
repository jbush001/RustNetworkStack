

pub struct NetworkPacket {
    pub data: [u8; 2048],
    pub length: i32,
    pub offset: i32,
}

pub fn alloc() -> NetworkPacket {
    NetworkPacket {
        data: [0; 2048],
        length: 0,
        offset: 0
    }
}
