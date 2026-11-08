//! Primitive readers for the LatinIME binary dictionary format.
//!
//! All integers are big-endian (see `ByteArrayUtils` in the C++ source).  Every
//! reader here is total: a truncated or corrupt dictionary yields zeros instead
//! of panicking, because a panic inside a JNI call would take the keyboard down.

/// First byte value that is stored as a literal ISO-8859-1 code point.
pub const MINIMUM_ONE_BYTE_CHARACTER_VALUE: u8 = 0x20;
/// Terminates a variable-length code point string.
pub const CHARACTER_ARRAY_TERMINATOR: u8 = 0x1F;

#[inline]
pub fn u8_at(data: &[u8], pos: usize) -> u8 {
    match data.get(pos) {
        Some(v) => *v,
        None => 0,
    }
}

#[inline]
pub fn read_u8(data: &[u8], pos: &mut usize) -> u8 {
    let v = u8_at(data, *pos);
    *pos = pos.wrapping_add(1);
    v
}

#[inline]
pub fn read_u16(data: &[u8], pos: &mut usize) -> u32 {
    let v = ((u8_at(data, *pos) as u32) << 8) | (u8_at(data, pos.wrapping_add(1)) as u32);
    *pos = pos.wrapping_add(2);
    v
}

#[inline]
pub fn read_u24(data: &[u8], pos: &mut usize) -> u32 {
    let v = ((u8_at(data, *pos) as u32) << 16)
        | ((u8_at(data, pos.wrapping_add(1)) as u32) << 8)
        | (u8_at(data, pos.wrapping_add(2)) as u32);
    *pos = pos.wrapping_add(3);
    v
}

#[inline]
pub fn read_u32(data: &[u8], pos: &mut usize) -> u32 {
    let v = ((u8_at(data, *pos) as u32) << 24)
        | ((u8_at(data, pos.wrapping_add(1)) as u32) << 16)
        | ((u8_at(data, pos.wrapping_add(2)) as u32) << 8)
        | (u8_at(data, pos.wrapping_add(3)) as u32);
    *pos = pos.wrapping_add(4);
    v
}

/// 24-bit *signed* value using the "sign in the first bit" scheme of the format:
/// values below `0x80` are positive and stored in three bytes, values with the
/// high bit set are negative and stored in ... one byte plus two of magnitude.
#[inline]
pub fn read_sint24(data: &[u8], pos: &mut usize) -> i32 {
    let first = u8_at(data, *pos);
    if first < 0x80 {
        read_u24(data, pos) as i32
    } else {
        *pos = pos.wrapping_add(1);
        let rest = read_u16(data, pos);
        -((((first & 0x7F) as i32) << 16) | rest as i32)
    }
}

#[inline]
pub fn read_uint(data: &[u8], size: usize, pos: &mut usize) -> u32 {
    match size {
        1 => read_u8(data, pos) as u32,
        2 => read_u16(data, pos),
        3 => read_u24(data, pos),
        4 => read_u32(data, pos),
        _ => 0,
    }
}

/// Reads one code point, honouring the optional single-byte code point table.
///
/// `table` maps `first_byte - 0x20` back to the original (frequent) code point.
#[inline]
pub fn read_code_point(data: &[u8], table: Option<&[i32]>, pos: &mut usize) -> i32 {
    let first = u8_at(data, *pos);
    if first < MINIMUM_ONE_BYTE_CHARACTER_VALUE {
        if first == CHARACTER_ARRAY_TERMINATOR {
            *pos = pos.wrapping_add(1);
            super::NOT_A_CODE_POINT
        } else {
            read_u24(data, pos) as i32
        }
    } else {
        *pos = pos.wrapping_add(1);
        match table {
            Some(t) => {
                let index = (first - MINIMUM_ONE_BYTE_CHARACTER_VALUE) as usize;
                match t.get(index) {
                    Some(cp) => *cp,
                    None => first as i32,
                }
            }
            None => first as i32,
        }
    }
}

/// Reads a terminated code point string into `out`; returns its length.
pub fn read_string(
    data: &[u8],
    table: Option<&[i32]>,
    max_len: usize,
    out: &mut [i32],
    pos: &mut usize,
) -> usize {
    let mut len = 0usize;
    loop {
        let cp = read_code_point(data, table, pos);
        if cp == super::NOT_A_CODE_POINT || len >= max_len || len >= out.len() {
            break;
        }
        out[len] = cp;
        len += 1;
    }
    len
}

/// Same as [`read_string`] but discards the content; returns the length.
pub fn skip_string(data: &[u8], max_len: usize, pos: &mut usize) -> usize {
    let mut len = 0usize;
    loop {
        let cp = read_code_point(data, None, pos);
        if cp == super::NOT_A_CODE_POINT || len >= max_len {
            break;
        }
        len += 1;
    }
    len
}

/// Reads a code point string and decodes it into a `String` (for header keys).
pub fn read_string_to_string(data: &[u8], max_len: usize, pos: &mut usize) -> String {
    let mut len = 0usize;
    let mut s = String::new();
    loop {
        let cp = read_code_point(data, None, pos);
        if cp == super::NOT_A_CODE_POINT || len >= max_len {
            break;
        }
        len += 1;
        if let Some(ch) = char::from_u32(cp as u32) {
            s.push(ch);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_big_endian_integers() {
        let data = [0x9B, 0xC1, 0x3A, 0xFE, 0x01, 0x02];
        let mut pos = 0;
        assert_eq!(read_u32(&data, &mut pos), 0x9BC1_3AFE);
        assert_eq!(reader_u16(&data, 4), 0x0102);
    }

    fn reader_u16(data: &[u8], at: usize) -> u32 {
        u16::from_be_bytes([data[at], data[at + 1]]) as u32
    }

    #[test]
    fn reads_one_and_three_byte_codepoints() {
        let data = [0x41, 0x01, 0x02, 0x03, 0x1F];
        let mut pos = 0;
        assert_eq!(read_code_point(&data, None, &mut pos), 0x41);
        assert_eq!(read_code_point(&data, None, &mut pos), 0x010203);
        assert_eq!(read_code_point(&data, None, &mut pos), -1);
    }

    #[test]
    fn reads_signed_offsets() {
        // Positive: 0x00 0x00 0x10 == 16
        let pos_data = [0x00, 0x00, 0x10];
        let mut pos = 0;
        assert_eq!(read_sint24(&pos_data, &mut pos), 16);

        // Negative: 0x80 | 0x7F, then two magnitude bytes => -(0x7F << 16 | 0x00 0x10)
        let neg_data = [0x80, 0x00, 0x10];
        let mut pos = 0;
        assert_eq!(read_sint24(&neg_data, &mut pos), -16);
    }
}
