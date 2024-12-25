

pub struct NetworkPacket {
    pub data: [u8; 2048],
    pub length: u32,
    pub offset: u32,
}

pub fn alloc() -> NetworkPacket {
    NetworkPacket {
        data: [0; 2048],
        length: 0,
        offset: 0
    }
}
