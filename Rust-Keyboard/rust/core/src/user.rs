//! The personalisation store.
//!
//! The C++ engine learns inside a *dynamic version 4 dictionary*: a Patricia
//! trie plus a hash-array-mapped trie that are rewritten (and garbage collected,
//! and compacted) as the user types.  That is a lot of machinery, and it costs
//! a full extra dictionary in RAM (the buffer is kept in heap memory and can
//! grow to several megabytes) for what is, in practice, a few thousand short
//! words.
//!
//! This module replaces it with an append-only log and a flat, sorted-in-place
//! record list that is capped at a configurable number of entries:
//!
//! ```text
//! "KBP1" | u16 version | u32 record count
//! record: u8 word length, u32[length] code points,
//!         u32 context hash, u8 probability, u8 flags, u32 timestamp
//! ```
//!
//! Worst-case RAM for the default cap of 2048 entries with 24 characters each
//! is ~256 KiB, and it is all in one contiguous `Vec` that never grows past the
//! cap.  Typical dictionaries use well under a tenth of that.

use std::fs;
use std::io::Write;

pub const MAGIC: &[u8; 4] = b"KBP1";
pub const FORMAT_VERSION: u16 = 1;
pub const DEFAULT_MAX_RECORDS: usize = 2048;
pub const MAX_WORD_CODEPOINTS: usize = 48;

pub const FLAG_NOT_A_WORD: u8 = 0x02;
pub const FLAG_POSSIBLY_OFFENSIVE: u8 = 0x04;
pub const FLAG_BLACKLISTED: u8 = 0x08;

const LOG_SUFFIX: &str = "kb_user.log";

#[derive(Clone)]
pub struct Record {
    pub word: Vec<i32>,
    /// FNV-1a hash of the preceding words; 0 means "no context".
    pub context: u32,
    pub probability: i32,
    pub flags: u8,
    pub timestamp: u32,
}

impl Record {
    #[allow(dead_code)]
    fn is_offensive(&self) -> bool {
        self.flags & FLAG_POSSIBLY_OFFENSIVE != 0
    }
}

/// A user dictionary: bounded, flat, and flushed lazily.
pub struct UserDict {
    path: String,
    records: Vec<Record>,
    dirty: bool,
    max_records: usize,
    clock: u32,
}

impl UserDict {
    /// Opens (or creates in memory) the log that belongs to `dict_path`.
    pub fn open(dict_path: &str) -> UserDict {
        let path = log_path_for(dict_path);
        let mut dict = UserDict {
            path,
            records: Vec::new(),
            dirty: false,
            max_records: DEFAULT_MAX_RECORDS,
            clock: 1,
        };
        if let Ok(bytes) = fs::read(&dict.path) {
            dict.parse(&bytes);
        }
        dict
    }

    pub fn empty(dict_path: &str) -> UserDict {
        UserDict {
            path: log_path_for(dict_path),
            records: Vec::new(),
            dirty: false,
            max_records: DEFAULT_MAX_RECORDS,
            clock: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Rebinds the log to the dictionary path handed over by `flushNative`.
    pub fn set_path(&mut self, dict_path: &str) {
        self.path = log_path_for(dict_path);
    }

    /// True when the log has grown enough that a compaction is worthwhile.
    pub fn needs_gc(&self) -> bool {
        self.records.len() >= self.max_records
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Records (or strengthens) `word`, returning the new probability.
    pub fn add_word(&mut self, word: &[i32], probability: i32, flags: u8, context: u32) -> i32 {
        if word.is_empty() || word.len() > MAX_WORD_CODEPOINTS {
            return probability;
        }
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        for record in self.records.iter_mut() {
            if record.context == context && record.word.as_slice() == word {
                // Typing the same word again makes it more likely.
                let boosted = (record.probability + 24).min(255);
                record.probability = boosted;
                record.flags = flags;
                record.timestamp = clock;
                self.dirty = true;
                return boosted;
            }
        }
        if self.records.len() >= self.max_records {
            self.evict_one();
        }
        self.records.push(Record {
            word: word.to_vec(),
            context,
            probability: probability.clamp(0, 255),
            flags,
            timestamp: clock,
        });
        self.dirty = true;
        probability
    }

    pub fn remove_word(&mut self, word: &[i32]) -> bool {
        let before = self.records.len();
        self.records.retain(|record| record.word.as_slice() != word);
        let changed = self.records.len() != before;
        if changed {
            self.dirty = true;
        }
        changed
    }

    pub fn lookup(&self, word: &[i32], context: u32) -> Option<i32> {
        for record in &self.records {
            if record.context == context && record.word.as_slice() == word {
                return Some(record.probability);
            }
        }
        None
    }

    /// Best probability for `word` in any context.
    pub fn lookup_any_context(&self, word: &[i32]) -> Option<i32> {
        let mut best: Option<i32> = None;
        for record in &self.records {
            if record.word.as_slice() == word {
                best = Some(match best {
                    Some(current) => current.max(record.probability),
                    None => record.probability,
                });
            }
        }
        best
    }

    pub fn word_flags(&self, word: &[i32]) -> u8 {
        for record in &self.records {
            if record.word.as_slice() == word {
                return record.flags;
            }
        }
        0
    }

    /// Calls `f` for every record whose word starts with `prefix`.
    pub fn for_each_prefixed<F: FnMut(&[i32], i32, u8)>(&self, prefix: &[i32], mut f: F) {
        for record in &self.records {
            if record.word.len() < prefix.len() {
                continue;
            }
            let mut matches = true;
            for index in 0..prefix.len() {
                if record.word[index] != prefix[index] {
                    matches = false;
                    break;
                }
            }
            if matches {
                f(&record.word, record.probability, record.flags);
            }
        }
    }

    pub fn words(&self) -> impl Iterator<Item = &Record> {
        self.records.iter()
    }

    /// Writes the log to disk; returns `false` when the write failed.
    pub fn flush(&mut self) -> bool {
        let mut bytes: Vec<u8> = Vec::with_capacity(self.records.len() * 24 + 16);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(self.records.len() as u32).to_le_bytes());
        for record in &self.records {
            bytes.push(record.word.len() as u8);
            for code_point in &record.word {
                bytes.extend_from_slice(&(*code_point as u32).to_le_bytes());
            }
            bytes.extend_from_slice(&record.context.to_le_bytes());
            bytes.push(record.probability.clamp(0, 255) as u8);
            bytes.push(record.flags);
            bytes.extend_from_slice(&record.timestamp.to_le_bytes());
        }

        if let Some(parent) = std::path::Path::new(&self.path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let temporary = format!("{}.tmp", self.path);
        let write_result = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        if write_result.is_err() {
            return false;
        }
        if fs::rename(&temporary, &self.path).is_err() {
            let _ = fs::remove_file(&temporary);
            return false;
        }
        self.dirty = false;
        true
    }

    /// Drops the least recently used quarter of the records.
    fn evict_one(&mut self) {
        if self.records.is_empty() {
            return;
        }
        let mut oldest_index = 0usize;
        let mut oldest_timestamp = u32::MAX;
        for (index, record) in self.records.iter().enumerate() {
            if record.timestamp < oldest_timestamp {
                oldest_timestamp = record.timestamp;
                oldest_index = index;
            }
        }
        self.records.swap_remove(oldest_index);
        self.dirty = true;
    }

    fn parse(&mut self, bytes: &[u8]) {
        if bytes.len() < 10 || &bytes[0..4] != MAGIC {
            return;
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != FORMAT_VERSION {
            return;
        }
        let count = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        let mut position = 10usize;
        let mut parsed = 0usize;
        while parsed < count && parsed < self.max_records {
            if position >= bytes.len() {
                break;
            }
            let length = bytes[position] as usize;
            position += 1;
            if length == 0 || length > MAX_WORD_CODEPOINTS {
                break;
            }
            if position + length * 4 + 10 > bytes.len() {
                break;
            }
            let mut word = Vec::with_capacity(length);
            for _ in 0..length {
                let value = u32::from_le_bytes([
                    bytes[position],
                    bytes[position + 1],
                    bytes[position + 2],
                    bytes[position + 3],
                ]);
                word.push(value as i32);
                position += 4;
            }
            let context = u32::from_le_bytes([
                bytes[position],
                bytes[position + 1],
                bytes[position + 2],
                bytes[position + 3],
            ]);
            position += 4;
            let probability = bytes[position] as i32;
            position += 1;
            let flags = bytes[position];
            position += 1;
            let timestamp = u32::from_le_bytes([
                bytes[position],
                bytes[position + 1],
                bytes[position + 2],
                bytes[position + 3],
            ]);
            position += 4;
            self.clock = self.clock.max(timestamp).wrapping_add(1);
            self.records.push(Record {
                word,
                context,
                probability,
                flags,
                timestamp,
            });
            parsed += 1;
        }
    }
}

/// FNV-1a over the code points; stable across runs and versions on purpose.
pub fn hash_context(words: &[&[i32]]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for word in words {
        for code_point in word.iter() {
            for byte in code_point.to_le_bytes() {
                hash ^= byte as u32;
                hash = hash.wrapping_mul(0x0100_0193);
            }
        }
        hash ^= 0xFF;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn log_path_for(dict_path: &str) -> String {
    let path = std::path::Path::new(dict_path);
    if path.is_dir() {
        format!("{}/{}", dict_path, LOG_SUFFIX)
    } else {
        format!("{}.{}", dict_path, LOG_SUFFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!("kb-core-test-{}-{}", std::process::id(), name));
        path.to_string_lossy().to_string()
    }

    #[test]
    fn adds_and_looks_up() {
        let mut user = UserDict::empty(&temp_path("add"));
        assert_eq!(user.len(), 0);
        user.add_word(&[0x68, 0x69], 100, 0, 0);
        assert_eq!(user.lookup(&[0x68, 0x69], 0), Some(100));
        assert_eq!(user.lookup(&[0x68, 0x69], 7), None);
    }

    #[test]
    fn repeated_word_gets_stronger() {
        let mut user = UserDict::empty(&temp_path("boost"));
        user.add_word(&[0x61], 100, 0, 0);
        let second = user.add_word(&[0x61], 100, 0, 0);
        assert!(second > 100);
        assert_eq!(user.len(), 1);
    }

    #[test]
    fn roundtrips_through_disk() {
        let path = temp_path("roundtrip");
        {
            let mut user = UserDict::empty(&path);
            user.add_word(&[0x61, 0x62], 200, FLAG_POSSIBLY_OFFENSIVE, 9);
            user.add_word(&[0x63], 10, 0, 0);
            assert!(user.flush());
        }
        let mut reopened = UserDict::empty(&path);
        if let Ok(bytes) = fs::read(log_path_for(&path)) {
            reopened.parse(&bytes);
        }
        assert_eq!(reopened.len(), 2);
        assert_eq!(reopened.lookup(&[0x61, 0x62], 9), Some(200));
        let _ = fs::remove_file(log_path_for(&path));
    }

    #[test]
    fn prefix_search_works() {
        let mut user = UserDict::empty(&temp_path("prefix"));
        user.add_word(&[0x61, 0x62, 0x63], 100, 0, 0);
        user.add_word(&[0x61, 0x62, 0x64], 100, 0, 0);
        user.add_word(&[0x78], 100, 0, 0);
        let mut count = 0;
        user.for_each_prefixed(&[0x61, 0x62], |_word, _probability, _flags| {
            count += 1;
        });
        assert_eq!(count, 2);
    }

    #[test]
    fn context_hash_is_stable() {
        let a = hash_context(&[&[0x61]]);
        let b = hash_context(&[&[0x61]]);
        let c = hash_context(&[&[0x62]]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
