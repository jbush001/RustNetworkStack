

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
