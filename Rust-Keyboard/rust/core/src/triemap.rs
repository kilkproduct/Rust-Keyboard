//! The hash-array-mapped trie that stores unigram and n-gram probabilities.
//!
//! Version 4 dictionaries keep the probability of a word outside the trie: the
//! PtNode stores a 4-byte *terminal id*, and the value lives in the language
//! model map keyed by that id.  Higher levels of the same map hold n-gram
//! probabilities, keyed by the current word id under a per-context bitmap index.
//!
//! Layout, all big endian, entries are 7 bytes (4 + 3):
//!
//! ```text
//!   0 .. 128            : the root hash table (32 buckets * 4 bytes)
//!   128 + i * 7         : field0 of entry i
//!   128 + i * 7 + 4     : field1 of entry i
//! ```
//!
//! Field1 carries two flag bits; when neither is set the entry is a *bitmap*
//! entry (`field0` = bitmap, `field1` = index of the child table).

use crate::bytes;

const FIELD0_SIZE: usize = 4;
const ENTRY_SIZE: usize = 7;
const ROOT_BITMAP_ENTRY_POS: usize = 32 * FIELD0_SIZE;

const VALUE_FLAG: u32 = 0x40_0000;
const VALUE_MASK: u32 = 0x3F_FFFF;
const INVALID_VALUE_IN_KEY_VALUE_ENTRY: u32 = VALUE_MASK;
const TERMINAL_LINK_FLAG: u32 = 0x80_0000;
const TERMINAL_LINK_MASK: u32 = 0x7F_FFFF;
const NUM_OF_BITS_USED_FOR_ONE_LEVEL: u32 = 5;
const LABEL_MASK: u32 = 0x1F;
/// Depth guard: 4 levels already address ~1M keys.
const MAX_LEVEL: u32 = 8;

pub const INVALID_INDEX: i32 = -1;

/// Outcome of a lookup.
#[derive(Clone, Copy, Debug)]
pub struct TrieMapResult {
    pub value: u64,
    pub is_valid: bool,
    pub next_level_bitmap_entry_index: i32,
}

impl TrieMapResult {
    pub fn invalid() -> TrieMapResult {
        TrieMapResult {
            value: 0,
            is_valid: false,
            next_level_bitmap_entry_index: INVALID_INDEX,
        }
    }
}

#[inline]
fn field0(data: &[u8], entry_index: usize) -> u32 {
    let mut pos = ROOT_BITMAP_ENTRY_POS + entry_index * ENTRY_SIZE;
    bytes::read_u32(data, &mut pos)
}

#[inline]
fn field1(data: &[u8], entry_index: usize) -> u32 {
    let mut pos = ROOT_BITMAP_ENTRY_POS + entry_index * ENTRY_SIZE + FIELD0_SIZE;
    bytes::read_u24(data, &mut pos)
}

#[inline]
fn is_bitmap_entry(data1: u32) -> bool {
    (data1 & VALUE_FLAG) == 0 && (data1 & TERMINAL_LINK_FLAG) == 0
}

/// The hash function of the map: it spreads each byte of the key across the
/// five bits used per level.
#[inline]
pub fn shuffled_key(key: u32) -> u32 {
    let mut shuffled: u32 = 0;
    let mut i = 0u32;
    while i < 4 {
        let piece = (key >> (i * 8)) & 0xFF;
        shuffled ^= ((piece ^ (piece << 7) ^ (piece << 14) ^ (piece << 21)) & 0x1111_1111) << i;
        i += 1;
    }
    shuffled
}

#[inline]
fn label(hashed_key: u32, level: u32) -> u32 {
    let shift = NUM_OF_BITS_USED_FOR_ONE_LEVEL * level;
    if shift >= 32 {
        return 0;
    }
    (hashed_key >> shift) & LABEL_MASK
}

#[inline]
fn popcount_below(bitmap: u32, index: u32) -> usize {
    if index == 0 {
        return 0;
    }
    let mask = (1u32 << index) - 1;
    (bitmap & mask).count_ones() as usize
}

/// Looks `key` up in the table rooted at `bitmap_entry_index`.
pub fn get(data: &[u8], key: i32, bitmap_entry_index: i32) -> TrieMapResult {
    if bitmap_entry_index < 0 {
        return TrieMapResult::invalid();
    }
    let hashed = shuffled_key(key as u32);
    let mut bitmap = field0(data, bitmap_entry_index as usize);
    let mut table_index = field1(data, bitmap_entry_index as usize) as usize;
    let mut level = 0u32;

    loop {
        if level > MAX_LEVEL {
            return TrieMapResult::invalid();
        }
        let label = label(hashed, level);
        if bitmap & (1u32 << label) == 0 {
            return TrieMapResult::invalid();
        }
        let entry_index = table_index + popcount_below(bitmap, label);
        let data0 = field0(data, entry_index);
        let data1 = field1(data, entry_index);

        if is_bitmap_entry(data1) {
            // Descend into the next level.
            bitmap = data0;
            table_index = data1 as usize;
            level += 1;
            continue;
        }

        if data0 != key as u32 {
            // A hash collision with a different key.
            return TrieMapResult::invalid();
        }

        if data1 & TERMINAL_LINK_FLAG != 0 {
            let value_entry_index = (data1 & TERMINAL_LINK_MASK) as usize;
            let low = field0(data, value_entry_index) as u64;
            let high = field1(data, value_entry_index) as u64;
            return TrieMapResult {
                value: (low << 24) | high,
                is_valid: true,
                next_level_bitmap_entry_index: (value_entry_index + 1) as i32,
            };
        }

        let value = data1 & VALUE_MASK;
        if value == INVALID_VALUE_IN_KEY_VALUE_ENTRY {
            return TrieMapResult::invalid();
        }
        return TrieMapResult {
            value: value as u64,
            is_valid: true,
            next_level_bitmap_entry_index: INVALID_INDEX,
        };
    }
}

/// Returns the bitmap index of the sub-map stored under `key`, if any.
pub fn next_level_bitmap_entry_index(data: &[u8], key: i32, bitmap_entry_index: i32) -> i32 {
    let result = get(data, key, bitmap_entry_index);
    if !result.is_valid {
        return INVALID_INDEX;
    }
    result.next_level_bitmap_entry_index
}

/// Decodes the packed probability entry of a non-historical dictionary:
/// `(flags << 8) | probability`.
#[inline]
pub fn decode_probability_entry(value: u64) -> (u8, i32) {
    let flags = ((value >> 8) & 0xFF) as u8;
    let probability = (value & 0xFF) as i32;
    (flags, probability)
}

/// Decodes a historical entry: `flags, timestamp, level, count`.
///
/// Field sizes are 1 + 4 + 0 + 2 bytes inside a 56-bit value.
#[inline]
pub fn decode_historical_entry(value: u64) -> (u8, i32, i32, i32) {
    let flags = ((value >> 48) & 0xFF) as u8;
    let timestamp = ((value >> 16) & 0xFFFF_FFFF) as i32;
    let count = (value & 0xFFFF) as i32;
    (flags, timestamp, 0, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_spreads_bits() {
        assert_eq!(shuffled_key(0), 0);
        assert_ne!(shuffled_key(1), shuffled_key(2));
    }

    #[test]
    fn empty_map_has_no_entries() {
        // An all-zero buffer behaves like an empty map: the root bitmap is 0.
        let data = vec![0u8; 4096];
        let result = get(&data, 42, 0);
        assert!(!result.is_valid);
    }

    #[test]
    fn decodes_probability_entry() {
        let (flags, probability) = decode_probability_entry((0x02u64 << 8) | 200);
        assert_eq!(flags, 0x02);
        assert_eq!(probability, 200);
    }
}
