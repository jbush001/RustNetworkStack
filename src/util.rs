

// Compute one's complement sum, per RFV 1071
// https://datatracker.ietf.org/doc/html/rfc1071
pub fn compute_checksum(buffer: &[u8]) -> u16 {
    let mut checksum: u32 = 0;

    let mut i = 0;
    while i < buffer.len() - 1 {
        checksum += u16::from_be_bytes([buffer[i], buffer[i + 1]]) as u32;
        i += 2
    }

    if i < buffer.len() {
        checksum += buffer[i] as u32;
    }

    while checksum > 0xffff {
        checksum = (checksum & 0xffff) + (checksum >> 16);
    }

    (checksum ^ 0xffff) as u16
}

pub fn get_be16(buffer: &[u8]) -> u16 {
    (((buffer[0] as u16) << 8) | buffer[1] as u16) as u16
}

pub fn get_be32(buffer: &[u8]) -> u32 {
    ((buffer[0] as u32) << 24) |
    ((buffer[1] as u32) << 16) |
    ((buffer[2] as u32) << 8) |
    buffer[3] as u32
}

pub fn set_be16(buffer: &mut [u8], value: u16) {
    buffer[0] = ((value >> 8) & 0xff) as u8;
    buffer[1] = (value & 0xff) as u8;
}

pub fn set_be32(buffer: &mut [u8], value: u32) {
    buffer[0] = ((value >> 24) & 0xff) as u8;
    buffer[1] = ((value >> 16) & 0xff) as u8;
    buffer[2] = ((value >> 8) & 0xff) as u8;
    buffer[3] = (value & 0xff) as u8;
}

pub fn ip_to_str(addr: u32) -> String {
    format!("{}.{}.{}.{}",
        (addr >> 24) & 0xff,
        (addr >> 16) & 0xff,
        (addr >> 8) & 0xff,
        addr & 0xff)
}