//! Dictionary header parsing.
//!
//! The header is the same for every dictionary version:
//!
//! ```text
//! magic number  : 4 bytes, 0x9B 0xC1 0x3A 0xFE
//! version       : 2 bytes
//! flags         : 2 bytes
//! header size   : 4 bytes, big endian, includes the 12 bytes above
//! attributes    : repeated { code point string key, code point string value }
//! ```

use crate::bytes;

pub const MAGIC_NUMBER: u32 = 0x9BC1_3AFE;
pub const HEADER_MINIMUM_SIZE: usize = 12;

/// Bounds on the attribute strings, matching `HeaderReadWriteUtils`.
const MAX_ATTRIBUTE_KEY_LENGTH: usize = 256;
const MAX_ATTRIBUTE_VALUE_LENGTH: usize = 2048;

pub const KEY_CODE_POINT_TABLE: &str = "codePointTable";
pub const KEY_HAS_HISTORICAL_INFO: &str = "HAS_HISTORICAL_INFO";
pub const KEY_LOCALE: &str = "locale";
pub const KEY_MULTIPLE_WORDS_DEMOTION_RATE: &str = "MULTIPLE_WORDS_DEMOTION_RATE";
pub const KEY_REQUIRES_GERMAN_UMLAUT_PROCESSING: &str = "REQUIRES_GERMAN_UMLAUT_PROCESSING";
pub const KEY_EXTENDED_REGION_SIZE: &str = "EXTENDED_REGION_SIZE";
pub const KEY_UNIGRAM_COUNT: &str = "UNIGRAM_COUNT";
pub const KEY_BIGRAM_COUNT: &str = "BIGRAM_COUNT";

/// Dictionary format revisions we understand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DictVersion {
    /// Version 2.02: single-buffer static dictionary with inline probabilities.
    V202,
    /// Version 399: only used by tests.
    V4OnlyForTesting,
    /// Version 4.02.
    V402,
    /// Version 4.03, the version every current dictionary uses.
    V403,
}

impl DictVersion {
    pub fn from_number(version: u32) -> Option<DictVersion> {
        match version {
            202 => Some(DictVersion::V202),
            399 => Some(DictVersion::V4OnlyForTesting),
            402 => Some(DictVersion::V402),
            403 => Some(DictVersion::V403),
            _ => None,
        }
    }

    /// Version 4 keeps the trie, the terminal table and the language model in
    /// separate buffers and stores terminal ids instead of inline probabilities.
    pub fn is_v4(self) -> bool {
        match self {
            DictVersion::V4OnlyForTesting | DictVersion::V402 | DictVersion::V403 => true,
            DictVersion::V202 => false,
        }
    }

    pub fn is_dynamic(self) -> bool {
        match self {
            DictVersion::V4OnlyForTesting | DictVersion::V402 | DictVersion::V403 => true,
            DictVersion::V202 => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Header {
    pub number: u32,
    pub version: DictVersion,
    pub flags: u16,
    pub size: usize,
    pub attributes: Vec<(String, Vec<i32>)>,
}

impl Header {
    pub fn parse(data: &[u8]) -> Option<Header> {
        if data.len() < HEADER_MINIMUM_SIZE {
            return None;
        }
        let mut pos = 0usize;
        let magic = bytes::read_u32(data, &mut pos);
        if magic != MAGIC_NUMBER {
            return None;
        }
        let number = bytes::read_u16(data, &mut pos);
        let version = DictVersion::from_number(number)?;
        let flags = bytes::read_u16(data, &mut pos) as u16;
        let size = bytes::read_u32(data, &mut pos) as usize;
        if size < HEADER_MINIMUM_SIZE || size > data.len() {
            return None;
        }

        let mut attributes: Vec<(String, Vec<i32>)> = Vec::new();
        let mut read_pos = 12usize;
        // Attribute values never use the code point table.
        while read_pos < size && attributes.len() < 64 {
            let key = bytes::read_string_to_string(data, MAX_ATTRIBUTE_KEY_LENGTH, &mut read_pos);
            let mut value = vec![0i32; MAX_ATTRIBUTE_VALUE_LENGTH];
            let value_len = bytes::read_string(
                data,
                None,
                MAX_ATTRIBUTE_VALUE_LENGTH,
                &mut value,
                &mut read_pos,
            );
            value.truncate(value_len);
            attributes.push((key, value));
        }

        Some(Header {
            number,
            version,
            flags,
            size,
            attributes,
        })
    }

    pub fn is_v4(&self) -> bool {
        self.version.is_v4()
    }

    fn attribute(&self, key: &str) -> Option<&Vec<i32>> {
        for entry in &self.attributes {
            if entry.0 == key {
                return Some(&entry.1);
            }
        }
        None
    }

    pub fn attribute_i32(&self, key: &str) -> Option<i32> {
        let value = self.attribute(key)?;
        let mut result: i64 = 0;
        let mut saw_digit = false;
        let mut negative = false;
        for cp in value {
            let ch = char::from_u32(*cp as u32)?;
            if ch == '-' && !saw_digit {
                negative = true;
            } else if let Some(digit) = ch.to_digit(10) {
                saw_digit = true;
                result = result * 10 + digit as i64;
            } else {
                break;
            }
        }
        if !saw_digit {
            return None;
        }
        if negative {
            result = -result;
        }
        if result > i32::MAX as i64 || result < i32::MIN as i64 {
            return None;
        }
        Some(result as i32)
    }

    pub fn attribute_bool(&self, key: &str) -> bool {
        match self.attribute(key) {
            Some(value) => !value.is_empty() && value[0] != ('0' as i32),
            None => false,
        }
    }

    pub fn has_historical_info(&self) -> bool {
        self.attribute_bool(KEY_HAS_HISTORICAL_INFO)
    }

    pub fn requires_german_umlaut_processing(&self) -> bool {
        self.attribute_bool(KEY_REQUIRES_GERMAN_UMLAUT_PROCESSING)
    }

    /// `true` for dictionaries that were built with exactly matched n-grams
    /// boosted, which is what the original engine reads to decide whether an
    /// exact match may be promoted above a statistically better word.
    pub fn should_boost_exact_matches(&self) -> bool {
        true
    }

    pub fn multiple_word_cost_multiplier(&self) -> f32 {
        let demotion_rate = self
            .attribute_i32(KEY_MULTIPLE_WORDS_DEMOTION_RATE)
            .unwrap_or(100);
        if demotion_rate <= 0 {
            return 1.0f32;
        }
        100.0f32 / demotion_rate as f32
    }

    /// The table that maps single-byte characters back to their real code point.
    pub fn code_point_table(&self) -> Option<Vec<i32>> {
        let value = self.attribute(KEY_CODE_POINT_TABLE)?;
        let mut table = Vec::with_capacity(value.len());
        for cp in value {
            table.push(*cp);
        }
        Some(table)
    }

    pub fn locale(&self) -> Option<String> {
        let value = self.attribute(KEY_LOCALE)?;
        let mut out = String::new();
        for cp in value {
            let ch = char::from_u32(*cp as u32)?;
            out.push(ch);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_header(version: u16, attributes: &[(&str, &str)]) -> Vec<u8> {
        let mut body: Vec<u8> = Vec::new();
        for (key, value) in attributes {
            for ch in key.chars() {
                body.push(ch as u8);
            }
            body.push(bytes::CHARACTER_ARRAY_TERMINATOR);
            for ch in value.chars() {
                body.push(ch as u8);
            }
            body.push(bytes::CHARACTER_ARRAY_TERMINATOR);
        }
        let size = 12 + body.len();
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&MAGIC_NUMBER.to_be_bytes());
        out.extend_from_slice(&version.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(size as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_v4_header_with_attributes() {
        let data = build_header(403, &[("locale", "en"), ("UNIGRAM_COUNT", "1234")]);
        let header = Header::parse(&data).expect("header should parse");
        assert_eq!(header.number, 403);
        assert!(header.is_v4());
        assert_eq!(header.locale().as_deref(), Some("en"));
        assert_eq!(header.attribute_i32("UNIGRAM_COUNT"), Some(1234));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut data = build_header(403, &[]);
        data[0] = 0;
        assert!(Header::parse(&data).is_none());
    }
}
