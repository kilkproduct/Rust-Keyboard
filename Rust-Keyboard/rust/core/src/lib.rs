//! Memory-tight rewrite of the keyboard's native dictionary / suggestion core.
//!
//! The C++ original (`native/jni/src`) keeps every dictionary in an in-memory,
//! growable heap buffer and mirrors the whole thing into a `PtNode` heap-trie
//! for the user's personal dictionary.  This crate keeps dictionaries where the
//! kernel already put them -- in the page cache -- via read-only `mmap`, and
//! works off borrowed slices (`&[u8]`) with fixed-size scratch buffers, so a
//! typical typing session allocates nothing at all after start-up.
//!
//! Layout of the crate:
//!
//! * [`bytes`]     - big-endian / variable-length primitives of the file format
//! * [`mmap`]      - the only `unsafe` heavy module: read-only file mapping
//! * [`header`]    - dictionary header (magic, version, attributes)
//! * [`ptrie`]     - Patricia-trie node & node-array decoding
//! * [`triemap`]   - the HAMT that stores unigram/ngram probabilities
//! * [`dict`]      - a dictionary: trie traversal + word attributes
//! * [`user`]      - compact personalisation log (replaces the dynamic dict)
//! * [`proximity`] - touch point -> candidate characters + spatial distance
//! * [`suggest`]   - the typed/gesture search that produces suggestions
//!
//! Nothing in this crate performs I/O other than reading the dictionary file
//! named by the caller, and nothing here can open a socket.

#![allow(clippy::needless_range_loop)]

pub mod bytes;
pub mod charutils;
pub mod dict;
pub mod header;
pub mod mmap;
pub mod proximity;
pub mod ptrie;
pub mod suggest;
pub mod triemap;
pub mod user;

/// Must be equal to `Constants.Dictionary.MAX_WORD_LENGTH` on the Java side.
pub const MAX_WORD_LENGTH: usize = 48;
/// Must be equal to `BinaryDictionary.MAX_RESULTS` on the Java side.
pub const MAX_RESULTS: usize = 40;
/// Must be equal to `ProximityInfo.MAX_PROXIMITY_CHARS_SIZE`.
pub const MAX_PROXIMITY_CHARS_SIZE: usize = 16;
/// Largest number of keys we keep proximity data for.
pub const MAX_KEY_COUNT_IN_A_KEYBOARD: usize = 256;
/// Largest number of previous words considered for n-gram context.
pub const MAX_PREV_WORD_COUNT_FOR_N_GRAM: usize = 4;

pub const NOT_A_PROBABILITY: i32 = -1;
pub const NOT_A_DICT_POS: i32 = -1;
pub const NOT_A_TERMINAL_ID: i32 = -1;
pub const NOT_A_CODE_POINT: i32 = -1;
pub const CODE_POINT_BEGINNING_OF_SENTENCE: i32 = 0x100000;
pub const MAX_PROBABILITY: i32 = 255;
pub const MAX_VALUE_FOR_WEIGHTING: i32 = 0x7FFF_FFFF;

/// Result kinds, must match `Dictionary.KIND_*` on the Java side.
pub const KIND_TYPED: i32 = 0;
pub const KIND_CORRECTION: i32 = 1;
pub const KIND_WHITELIST: i32 = 3;
pub const KIND_PREDICTION: i32 = 8;

/// Result attribute flags, must match `Dictionary.KIND_FLAG_*`.
pub const KIND_FLAG_POSSIBLY_OFFENSIVE: i32 = -0x8000_0000; // 0x80000000 as i32
pub const KIND_FLAG_EXACT_MATCH: i32 = 0x4000_0000;
pub const KIND_FLAG_EXACT_MATCH_WITH_INTENTIONAL_OMISSION: i32 = 0x2000_0000;
pub const KIND_FLAG_APPROPRIATE_FOR_AUTOCORRECTION: i32 = 0x1000_0000;
