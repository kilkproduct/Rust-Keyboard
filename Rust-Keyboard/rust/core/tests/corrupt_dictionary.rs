//! Corrupt dictionaries must not take the process down.
//!
//! A dictionary is a file the app did not write: it is downloaded, copied out
//! of an APK, or left half-flushed by a killed download. The reader is built
//! with `panic = "abort"`, so a single slice index panic inside a trie walk
//! kills the keyboard process mid-keystroke. These tests feed the reader
//! truncated headers, plausible-but-garbage tries and random mutations of a
//! well-formed file, and call every public read entry point on the result.
//! Reaching the end of a test is the assertion: any panic fails it.

use kb_core::dict::Dictionary;
use kb_core::header;
use kb_core::suggest::{self, SuggestedWord};

const V4_BUFFER_COUNT: usize = 7;

fn scratch(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("kb-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join(name).to_string_lossy().to_string()
}

/// Deterministic pseudo-random bytes: no dependency, same sequence every run so
/// a failure can be reproduced from the reported case.
fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        out.push(state as u8);
    }
    out
}

fn push_buffer(out: &mut Vec<u8>, body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
}

/// A version 4 file whose header is valid and whose trie is garbage. Every
/// buffer length is consistent, so `split_v4_buffers` accepts the file.
fn v4_with_garbage_trie(seed: u32) -> Vec<u8> {
    let mut data = header::serialize(403, &[("locale", "en_US")]);
    push_buffer(&mut data, &noise(seed, 512)); // trie
    push_buffer(&mut data, &noise(seed + 1, 64)); // terminal table
    push_buffer(&mut data, &noise(seed + 2, 96)); // language model
    for _ in 3..V4_BUFFER_COUNT {
        push_buffer(&mut data, &[]);
    }
    data
}

/// A version 2 file: the trie is everything after the header, so a truncated
/// download is a trie that ends in the middle of a node.
fn v2_with_garbage_trie(seed: u32) -> Vec<u8> {
    let mut data = header::serialize(202, &[("locale", "en_US")]);
    data.extend_from_slice(&noise(seed, 700));
    data
}

/// Every read path the JNI layer can reach on a keystroke.
fn exercise(dict: &Dictionary) {
    let mut results: Vec<SuggestedWord> = Vec::new();

    // No context, then a context; both walk the trie from the root.
    suggest::get_predictions(dict, &[], &mut results);
    assert!(results.len() <= kb_core::MAX_RESULTS);

    let context = vec![
        "the".chars().map(|c| c as i32).collect::<Vec<i32>>(),
        "quick".chars().map(|c| c as i32).collect::<Vec<i32>>(),
    ];
    suggest::get_predictions(dict, &context, &mut results);
    assert!(results.len() <= kb_core::MAX_RESULTS);

    // Typed lookups, including code points that are not on the keyboard.
    for word in ["a", "the", "keyboard", "zzzzzzzz"] {
        let code_points: Vec<i32> = word.chars().map(|c| c as i32).collect();
        let (terminal_id, node) = dict.find_word(&code_points);
        let _ = terminal_id;
        if let Some(node) = node {
            dict.for_each_child(&node, |_| true);
        }
        let _ = dict.get_probability(&code_points);
        let _ = dict.word_attributes(&[], 0);
        let _ = dict.get_probability(&[0x110000, -5, 0x7FFF_FFFF]);
    }

    // Nodes decoded straight out of the trie at positions the search would
    // never choose, the way a corrupt child offset does.
    let root = dict.root_array();
    for offset in 0..64usize {
        let position = root.first_node_pos.wrapping_add(offset);
        if let Some(node) = dict.node_at(position) {
            dict.for_each_child(&node, |_| true);
        }
        let _ = dict.array_at(position);
    }
}

fn open_and_exercise(bytes: &[u8], name: &str) {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write scratch dictionary");
    if let Some(dict) = Dictionary::open(&path, 0, 0, false) {
        exercise(&dict);
    }
    // The same bytes through the unmapped window path, which has to align the
    // offset down to a page boundary.
    if let Some(dict) = Dictionary::open(&path, 1, 0, false) {
        exercise(&dict);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn truncated_headers_are_rejected() {
    let data = v4_with_garbage_trie(1);
    for len in 0..16usize {
        let path = scratch(&format!("short-{}.dict", len));
        std::fs::write(&path, &data[..len]).expect("write");
        // Never panics; a header that cannot be parsed simply yields no dict.
        let _ = Dictionary::open(&path, 0, 0, false);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn version_4_with_garbage_trie_does_not_panic() {
    for seed in 0..4u32 {
        open_and_exercise(&v4_with_garbage_trie(seed), &format!("v4-{}.dict", seed));
    }
}

#[test]
fn version_2_with_garbage_trie_does_not_panic() {
    for seed in 0..4u32 {
        open_and_exercise(&v2_with_garbage_trie(seed), &format!("v2-{}.dict", seed));
    }
}

#[test]
fn every_prefix_of_a_valid_file_does_not_panic() {
    // A download that was cut off at an arbitrary byte offset: the header may
    // parse while a buffer length points past the end of the file.
    let data = v4_with_garbage_trie(99);
    for len in (0..data.len()).step_by(151) {
        open_and_exercise(&data[..len], &format!("prefix-{}.dict", len));
    }
}

#[test]
fn random_mutations_of_a_valid_file_do_not_panic() {
    let base = v4_with_garbage_trie(1234);
    for seed in 0..8u32 {
        let mut mutated = base.clone();
        let noise = noise(seed.wrapping_mul(2654435761), mutated.len());
        for (index, byte) in mutated.iter_mut().enumerate() {
            // Flip one byte in every 97: enough to break node flags, offsets
            // and buffer lengths while keeping the header mostly intact.
            if noise[index] % 97 == 0 {
                *byte ^= noise[(index + 7) % noise.len()];
            }
        }
        open_and_exercise(&mutated, &format!("mutated-{}.dict", seed));
    }
}

#[test]
fn corrupt_dictionary_reports_no_words() {
    // A buffer length that runs past the end of the file: the header parses but
    // the buffers do not line up, which is what a half-written download looks
    // like. The dictionary must be flagged, and it must answer with nothing
    // instead of walking the garbage that follows.
    let mut data = header::serialize(403, &[("locale", "en_US")]);
    data.extend_from_slice(&(1_000_000u32).to_be_bytes());
    data.extend_from_slice(&noise(3, 64));

    let path = scratch("corrupt.dict");
    std::fs::write(&path, &data).expect("write");
    let dict = Dictionary::open(&path, 0, 0, false).expect("header parses");
    assert!(dict.is_corrupted(), "a buffer past EOF must be flagged");

    let mut results: Vec<SuggestedWord> = Vec::new();
    suggest::get_predictions(&dict, &[], &mut results);
    assert!(results.is_empty(), "a corrupt dictionary suggests nothing");
    let _ = std::fs::remove_file(&path);
}
