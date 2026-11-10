//! A dictionary: the trie plus the buffers that hold its probabilities.
//!
//! Two on-disk shapes are handled transparently:
//!
//! * **directory dictionaries** (version 4, writable): `<dir>/<name>.header` and
//!   `<dir>/<name>.body`, where the body is a sequence of seven
//!   size-prefixed buffers,
//! * **single-file dictionaries** (bundled, read-only): one file with the header
//!   at the requested offset.
//!
//! Personalisation ("user dictionary") is not a version 4 dynamic dictionary
//! any more; see [`crate::user`].

use crate::header::{DictVersion, Header};
use crate::mmap::{Buf, DataSource};
use crate::ptrie::{self, Layout, Node};
use crate::triemap;
use crate::user::UserDict;
use crate::{MAX_PREV_WORD_COUNT_FOR_N_GRAM, MAX_WORD_LENGTH, NOT_A_PROBABILITY};

/// Upper bound on the children of one node. A real node array holds at most a
/// few hundred entries (one per distinct code point that continues the word).
const MAX_CHILDREN_PER_NODE: usize = 4096;

/// Number of size-prefixed buffers inside a version 4 body file:
/// trie, terminal table, language model map, language model counters,
/// shortcut lookup table, shortcut address table, shortcut content.
const V4_BUFFER_COUNT: usize = 7;
const V4_TRIE_BUFFER: usize = 0;
const V4_TERMINAL_TABLE_BUFFER: usize = 1;
const V4_LANGUAGE_MODEL_BUFFER: usize = 2;

const NOT_A_TERMINAL_ID: i32 = -1;

// Probability entry flags, see `Ver4DictConstants`.
const FLAG_REPRESENTS_BEGINNING_OF_SENTENCE: u8 = 0x1;
const FLAG_NOT_A_VALID_ENTRY: u8 = 0x2;
const FLAG_NOT_A_WORD: u8 = 0x4;
const FLAG_BLACKLISTED: u8 = 0x8;
const FLAG_POSSIBLY_OFFENSIVE: u8 = 0x10;

/// What the dictionary knows about a word in a given context.
#[derive(Clone, Copy, Debug)]
pub struct WordAttributes {
    pub probability: i32,
    pub is_blacklisted: bool,
    pub is_not_a_word: bool,
    pub is_possibly_offensive: bool,
    pub represents_beginning_of_sentence: bool,
}

impl WordAttributes {
    pub fn invalid() -> WordAttributes {
        WordAttributes {
            probability: NOT_A_PROBABILITY,
            is_blacklisted: false,
            is_not_a_word: false,
            is_possibly_offensive: false,
            represents_beginning_of_sentence: false,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.probability != NOT_A_PROBABILITY
    }
}

/// A `(count, first node position)` pair describing a PtNode array.
#[derive(Clone, Copy, Debug)]
pub struct NodeArray {
    pub count: usize,
    pub first_node_pos: usize,
}

pub struct Dictionary {
    body: DataSource,
    header: Header,
    trie: Buf,
    terminal_table: Buf,
    language_model: Buf,
    layout: Layout,
    code_point_table: Option<Vec<i32>>,
    is_v4: bool,
    has_historical_info: bool,
    /// Present for dictionaries the IME is allowed to learn from.
    pub user: Option<UserDict>,
    /// True when the file could not be interpreted and the dictionary is empty.
    pub corrupted: bool,
}

impl Dictionary {
    /// Opens `path`.
    ///
    /// * `offset`/`size` select a window inside a single-file dictionary; pass
    ///   `0, 0` to read the whole file.
    /// * `updatable` enables learning; the personalisation log then lives next
    ///   to the dictionary file.
    pub fn open(path: &str, offset: u64, size: usize, updatable: bool) -> Option<Dictionary> {
        let meta = std::fs::metadata(path).ok()?;
        if meta.is_dir() {
            return Dictionary::open_directory(path, updatable);
        }
        Dictionary::open_single_file(path, offset, size, updatable)
    }

    /// An empty dictionary that only carries a personalisation log.
    ///
    /// This is what `createOnMemoryNative` returns: the trie is empty and every
    /// lookup goes through the user log.
    pub fn new_in_memory(updatable: bool) -> Option<Dictionary> {
        let body = DataSource::from_vec(vec![0u8; 16]);
        let header = Header {
            number: 403,
            version: DictVersion::V403,
            flags: 0,
            size: 12,
            attributes: Vec::new(),
        };
        let mut dict = Dictionary {
            body,
            header,
            trie: Buf::new(12, 4),
            terminal_table: Buf::empty(),
            language_model: Buf::empty(),
            layout: Layout::V4,
            code_point_table: None,
            is_v4: true,
            has_historical_info: false,
            user: None,
            corrupted: false,
        };
        if updatable {
            dict.user = Some(UserDict::empty(""));
        }
        Some(dict)
    }

    fn open_directory(path: &str, updatable: bool) -> Option<Dictionary> {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())?;
        let header_path = format!("{}/{}.header", path, name);
        let body_path = format!("{}/{}.body", path, name);

        let header_source = DataSource::open(&header_path, 0, 0)?;
        let header = Header::parse(header_source.as_slice())?;
        if !header.is_v4() {
            return None;
        }
        let body = DataSource::open(&body_path, 0, 0)?;

        let mut dict = Dictionary {
            body,
            header,
            trie: Buf::empty(),
            terminal_table: Buf::empty(),
            language_model: Buf::empty(),
            layout: Layout::V4,
            code_point_table: None,
            is_v4: true,
            has_historical_info: false,
            user: None,
            corrupted: false,
        };
        dict.has_historical_info = dict.header.has_historical_info();
        if !dict.split_v4_buffers(0) {
            dict.corrupted = true;
            return Some(dict);
        }
        if updatable {
            dict.user = Some(UserDict::open(path));
        }
        Some(dict)
    }

    fn open_single_file(
        path: &str,
        offset: u64,
        size: usize,
        updatable: bool,
    ) -> Option<Dictionary> {
        let source = DataSource::open(path, offset, size)?;
        let header = Header::parse(source.as_slice())?;
        let table = header.code_point_table();
        // In a single file dictionary the size prefixed buffers start right
        // after the header; treating the header itself as the first buffer size
        // marks every bundled dictionary as corrupt.
        let data_offset = header.size;

        if header.is_v4() {
            let mut dict = Dictionary {
                body: source,
                header,
                trie: Buf::empty(),
                terminal_table: Buf::empty(),
                language_model: Buf::empty(),
                layout: Layout::V4,
                code_point_table: None,
                is_v4: true,
                has_historical_info: false,
                user: None,
                corrupted: false,
            };
            dict.has_historical_info = dict.header.has_historical_info();
            if !dict.split_v4_buffers(data_offset) {
                dict.corrupted = true;
            }
            if updatable {
                dict.user = Some(UserDict::open(path));
            }
            Some(dict)
        } else {
            // Version 2.02: the trie starts right after the header.
            let trie_offset = header.size;
            let mut dict = Dictionary {
                body: source,
                header,
                trie: Buf::new(trie_offset, 0),
                terminal_table: Buf::empty(),
                language_model: Buf::empty(),
                layout: Layout::V2Static,
                code_point_table: table,
                is_v4: false,
                has_historical_info: false,
                user: None,
                corrupted: false,
            };
            let total = dict.body.len();
            dict.trie = Buf::new(trie_offset, total.saturating_sub(trie_offset));
            if updatable {
                dict.user = Some(UserDict::open(path));
            }
            Some(dict)
        }
    }

    /// Writes a valid but empty version 4 dictionary to `path`.
    ///
    /// Android asks the native code to create the file for a user dictionary
    /// (`createEmptyDictFileNative`); the file has to be a real dictionary,
    /// otherwise opening it fails and every later personalisation call is lost.
    /// Words are *not* stored in this file: they live in the sidecar log owned by
    /// [`crate::user::UserDict`], so the file itself stays a valid empty trie.
    pub fn write_empty_v4(path: &str, version: u16, attributes: &[(&str, &str)]) -> bool {
        let mut bytes = crate::header::serialize(version, attributes);

        // Buffer 0: the trie, containing a single root node array with no nodes.
        let trie: [u8; 4] = [0, 0, 0, 0];
        bytes.extend_from_slice(&(trie.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&trie);
        // Buffers 1..7: terminal table, language model map and counters, shortcut
        // lookup table, address table and content, all empty.
        for _ in 1..V4_BUFFER_COUNT {
            bytes.extend_from_slice(&0u32.to_be_bytes());
        }

        let temporary = format!("{}.tmp", path);
        let write_result = std::fs::write(&temporary, &bytes);
        if write_result.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return false;
        }
        if std::fs::rename(&temporary, path).is_err() {
            let _ = std::fs::remove_file(&temporary);
            return false;
        }
        true
    }

    /// Splits a version 4 body into its seven size-prefixed buffers.
    fn split_v4_buffers(&mut self, data_offset: usize) -> bool {
        let data = self.body.as_slice();
        let mut buffers: [Buf; V4_BUFFER_COUNT] = [Buf::empty(); V4_BUFFER_COUNT];
        let mut position = data_offset;
        let mut index = 0usize;
        while position + 4 <= data.len() && index < V4_BUFFER_COUNT {
            let mut read = position;
            let buffer_size = crate::bytes::read_u32(data, &mut read) as usize;
            let start = read;
            let end = match start.checked_add(buffer_size) {
                Some(v) => v,
                None => return false,
            };
            if end > data.len() {
                return false;
            }
            buffers[index] = Buf::new(start, buffer_size);
            position = end;
            index += 1;
        }
        if index != V4_BUFFER_COUNT {
            return false;
        }
        self.trie = buffers[V4_TRIE_BUFFER];
        self.terminal_table = buffers[V4_TERMINAL_TABLE_BUFFER];
        self.language_model = buffers[V4_LANGUAGE_MODEL_BUFFER];
        true
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn version(&self) -> DictVersion {
        self.header.version
    }

    pub fn format_version(&self) -> i32 {
        self.header.number as i32
    }

    pub fn header_size(&self) -> usize {
        self.header.size
    }

    pub fn is_v4(&self) -> bool {
        self.is_v4
    }

    pub fn is_updatable(&self) -> bool {
        self.user.is_some()
    }

    /// True when the buffer sizes in the header did not add up. Such a
    /// dictionary is kept open (its personalisation log still works) but asking
    /// it for words returns nothing: walking a garbage trie would be walking
    /// unchecked offsets, and this crate is built with `panic = "abort"`.
    pub fn is_corrupted(&self) -> bool {
        self.corrupted
    }

    /// Asks the kernel to pull the trie into the page cache. Only useful
    /// before a walk that touches most of it - the next word predictions do.
    pub fn prefetch_trie(&self) {
        if self.trie.is_empty() {
            return;
        }
        let _ = self.body.willneed(self.trie.offset, self.trie.len);
    }

    fn trie_data(&self) -> &[u8] {
        self.trie.get(self.body.as_slice())
    }

    fn language_model_data(&self) -> &[u8] {
        self.language_model.get(self.body.as_slice())
    }

    /// The root PtNode array (always at position 0 of the trie buffer).
    pub fn root_array(&self) -> NodeArray {
        self.array_at(0)
    }

    pub fn array_at(&self, pos: usize) -> NodeArray {
        let data = self.trie_data();
        let mut read = pos;
        let count = ptrie::read_node_array_size(data, &mut read);
        NodeArray {
            count,
            first_node_pos: read,
        }
    }

    pub fn node_at(&self, pos: usize) -> Option<Node> {
        let layout = self.layout;
        let table = self.code_point_table.as_deref();
        let node = ptrie::read_node(self.trie_data(), pos, layout, table)?;
        if node.is_moved {
            ptrie::resolve_moved_node(self.trie_data(), node, layout, table)
        } else {
            Some(node)
        }
    }

    /// Iterates the children of `node`, calling `f` for each. Stops early when
    /// `f` returns `false`.
    pub fn for_each_child<F: FnMut(&Node) -> bool>(&self, node: &Node, mut f: F) {
        if node.children_pos == crate::NOT_A_DICT_POS {
            return;
        }
        let array = self.array_at(node.children_pos as usize);
        // The child count comes straight out of the file, and every child needs
        // at least one byte of the trie. Without this clamp a corrupt count made
        // one node look like it had 32 000 children, which turned a prediction
        // into a queue of millions of entries.
        let remaining = self.trie.len.saturating_sub(array.first_node_pos);
        let count = array.count.min(remaining).min(MAX_CHILDREN_PER_NODE);
        let mut pos = array.first_node_pos;
        let mut index = 0usize;
        while index < count {
            let child = match self.node_at(pos) {
                Some(c) => c,
                None => return,
            };
            if !f(&child) {
                return;
            }
            let next = child.sibling_pos;
            if next <= pos as i32 {
                return;
            }
            pos = next as usize;
            index += 1;
        }
    }

    /// Looks up the child of `node` that starts with `code_point`.
    pub fn find_child(&self, node: &Node, code_point: i32) -> Option<Node> {
        let mut found: Option<Node> = None;
        self.for_each_child(node, |child| {
            if child.char_at(0) == code_point {
                found = Some(*child);
                return false;
            }
            true
        });
        found
    }

    /// Probability of `terminal_id` in the context of `prev_ids`.
    ///
    /// For version 2 dictionaries the probability is stored inline and `prev_ids`
    /// is ignored.
    pub fn word_attributes(&self, prev_ids: &[i32], terminal_id: i32) -> WordAttributes {
        if terminal_id == NOT_A_TERMINAL_ID {
            return WordAttributes::invalid();
        }
        if !self.is_v4 {
            return WordAttributes {
                probability: terminal_id,
                ..WordAttributes::invalid()
            };
        }

        let data = self.language_model_data();
        if data.is_empty() {
            return WordAttributes::invalid();
        }

        // Walk down the n-gram levels as far as the previous words allow.
        let mut bitmap_indices = [0i32; MAX_PREV_WORD_COUNT_FOR_N_GRAM + 1];
        bitmap_indices[0] = 0;
        let mut max_prev_word_count = 0usize;
        for (index, prev_id) in prev_ids.iter().enumerate() {
            if index >= MAX_PREV_WORD_COUNT_FOR_N_GRAM {
                break;
            }
            let next = triemap::next_level_bitmap_entry_index(data, *prev_id, bitmap_indices[index]);
            if next == triemap::INVALID_INDEX {
                break;
            }
            max_prev_word_count = index + 1;
            bitmap_indices[index + 1] = next;
        }

        // Unigram entry carries the flags for the word itself.
        let unigram_entry = triemap::get(data, terminal_id, 0);
        let mut unigram_flags = 0u8;
        let mut unigram_probability = NOT_A_PROBABILITY;
        if unigram_entry.is_valid {
            if self.has_historical_info {
                let (flags, _timestamp, _level, count) =
                    triemap::decode_historical_entry(unigram_entry.value);
                unigram_flags = flags;
                if count == 0 {
                    return WordAttributes::invalid();
                }
                unigram_probability = count_to_probability(count);
            } else {
                let (flags, probability) = triemap::decode_probability_entry(unigram_entry.value);
                unigram_flags = flags;
                unigram_probability = probability;
            }
        }
        if unigram_flags & FLAG_NOT_A_VALID_ENTRY != 0 {
            return WordAttributes::invalid();
        }

        // Prefer the longest matching context (bigram, trigram, ...).
        let mut probability = unigram_probability;
        let mut context_flags = unigram_flags;
        let mut level = max_prev_word_count;
        loop {
            if level > 0 {
                let entry = triemap::get(data, terminal_id, bitmap_indices[level]);
                if entry.is_valid {
                    if self.has_historical_info {
                        let (flags, _ts, _lvl, count) =
                            triemap::decode_historical_entry(entry.value);
                        if flags & FLAG_NOT_A_VALID_ENTRY == 0 && count > 0 {
                            probability = count_to_probability(count);
                            context_flags = flags;
                        }
                    } else {
                        let (flags, ngram_probability) =
                            triemap::decode_probability_entry(entry.value);
                        if flags & FLAG_NOT_A_VALID_ENTRY == 0 {
                            probability = ngram_probability;
                            context_flags = flags;
                        }
                    }
                }
            }
            if level == 0 {
                break;
            }
            level -= 1;
        }

        WordAttributes {
            probability,
            is_blacklisted: context_flags & FLAG_BLACKLISTED != 0,
            is_not_a_word: context_flags & FLAG_NOT_A_WORD != 0,
            is_possibly_offensive: context_flags & FLAG_POSSIBLY_OFFENSIVE != 0,
            represents_beginning_of_sentence: context_flags
                & FLAG_REPRESENTS_BEGINNING_OF_SENTENCE
                != 0,
        }
    }

    /// Walks the trie to find `word`, returning `(terminal_id, node)`.
    ///
    /// `terminal_id` is `NOT_A_TERMINAL_ID` when the word is not a terminal; the
    /// returned node is then the deepest node that matched. For version 2
    /// dictionaries the "terminal id" field carries the inline probability.
    pub fn find_word(&self, word: &[i32]) -> (i32, Option<Node>) {
        let mut index = 0usize;
        let root = Node {
            pos: 0,
            children_pos: 0,
            ..Node::empty()
        };
        let mut current = root;
        let mut terminal_id = NOT_A_TERMINAL_ID;
        let mut last: Option<Node> = None;

        while index < word.len() {
            let wanted = word[index];
            let child = match self.find_child(&current, wanted) {
                Some(c) => c,
                None => return (NOT_A_TERMINAL_ID, last),
            };
            last = Some(child);
            // A node may carry several characters at once; consume them all.
            let mut matched = true;
            let mut char_index = 0usize;
            while char_index < child.char_count as usize {
                if index + char_index >= word.len()
                    || child.char_at(char_index) != word[index + char_index]
                {
                    matched = false;
                    break;
                }
                char_index += 1;
            }
            if !matched {
                return (NOT_A_TERMINAL_ID, last);
            }
            index += char_index;
            if child.is_terminal() {
                terminal_id = if self.is_v4 {
                    child.terminal_id
                } else {
                    child.probability
                };
            } else {
                terminal_id = NOT_A_TERMINAL_ID;
            }
            current = child;
        }

        (terminal_id, last)
    }

    /// Probability of an exact match, or `NOT_A_PROBABILITY`.
    pub fn get_probability(&self, word: &[i32]) -> i32 {
        let (terminal_id, _node) = self.find_word(word);
        if terminal_id == NOT_A_TERMINAL_ID {
            return NOT_A_PROBABILITY;
        }
        let attributes = self.word_attributes(&[], terminal_id);
        attributes.probability
    }

    /// Highest probability among the words that start with `word`.
    pub fn get_max_probability_of_exact_matches(&self, word: &[i32]) -> i32 {
        let (terminal_id, node) = self.find_word(word);
        let mut best = if terminal_id != NOT_A_TERMINAL_ID {
            self.word_attributes(&[], terminal_id).probability
        } else {
            NOT_A_PROBABILITY
        };
        if let Some(node) = node {
            let root = self.root_array();
            let _ = root;
            self.for_each_child(&node, |child| {
                if child.is_terminal() {
                    let id = if self.is_v4 {
                        child.terminal_id
                    } else {
                        child.probability
                    };
                    let probability = self.word_attributes(&[], id).probability;
                    if probability > best {
                        best = probability;
                    }
                }
                true
            });
        }
        best
    }

    /// True when the trie contains `word` as a word (not merely a prefix).
    pub fn contains(&self, word: &[i32]) -> bool {
        let (terminal_id, _node) = self.find_word(word);
        terminal_id != NOT_A_TERMINAL_ID
    }

    /// Iterates every word in the dictionary, in trie order.
    ///
    /// This is the read side of `getNextWordNative`; it is only used by
    /// dictionary tooling, so it allocates a bounded stack of node positions.
    pub fn for_each_word<F: FnMut(&[i32], i32, i32) -> bool>(&self, mut f: F) {
        let mut buffer = [0i32; MAX_WORD_LENGTH];
        let root = Node {
            pos: 0,
            children_pos: 0,
            ..Node::empty()
        };
        self.walk_words(&root, &mut buffer, 0, &mut f);
    }

    fn walk_words<F: FnMut(&[i32], i32, i32) -> bool>(
        &self,
        node: &Node,
        buffer: &mut [i32; MAX_WORD_LENGTH],
        length: usize,
        f: &mut F,
    ) -> bool {
        let array = if node.pos == 0 && node.children_pos == 0 {
            self.root_array()
        } else if node.children_pos != crate::NOT_A_DICT_POS {
            self.array_at(node.children_pos as usize)
        } else {
            return true;
        };

        let mut pos = array.first_node_pos;
        let mut index = 0usize;
        while index < array.count {
            let child = match self.node_at(pos) {
                Some(c) => c,
                None => return true,
            };
            let add = child.char_count as usize;
            if length + add > MAX_WORD_LENGTH {
                return true;
            }
            for i in 0..add {
                buffer[length + i] = child.char_at(i);
            }
            let new_length = length + add;
            if child.is_terminal() {
                let value = if self.is_v4 {
                    child.terminal_id
                } else {
                    child.probability
                };
                let probability = if self.is_v4 {
                    self.word_attributes(&[], value).probability
                } else {
                    value
                };
                if !f(&buffer[..new_length], value, probability) {
                    return false;
                }
            }
            if child.has_children() && !self.walk_words(&child, buffer, new_length, f) {
                return false;
            }
            let next = child.sibling_pos;
            if next <= pos as i32 {
                return true;
            }
            pos = next as usize;
            index += 1;
        }
        true
    }
}

/// Rough probability for dictionaries that only store counts (historical info).
fn count_to_probability(count: i32) -> i32 {
    if count <= 0 {
        return 0;
    }
    let scaled = 40 + (count.min(255) * 215) / 255;
    scaled.min(255)
}

/// Builds a version 2.02 trie in memory: a chain of PtNode arrays with
/// inline probabilities, which is the minimal shape the search reads.
#[cfg(test)]
pub mod test_trie {
    struct TNode {
        ch: i32,
        probability: Option<u8>,
        children: Vec<usize>,
    }

    pub fn build(words: &[(&str, u8)]) -> Vec<u8> {
        let mut arena: Vec<TNode> = vec![TNode {
            ch: 0,
            probability: None,
            children: Vec::new(),
        }];
        for (word, probability) in words {
            let mut current = 0usize;
            let chars: Vec<i32> = word.chars().map(|c| c as i32).collect();
            for (index, ch) in chars.iter().enumerate() {
                let existing = arena[current]
                    .children
                    .iter()
                    .copied()
                    .find(|id| arena[*id].ch == *ch);
                let next = match existing {
                    Some(id) => id,
                    None => {
                        arena.push(TNode {
                            ch: *ch,
                            probability: None,
                            children: Vec::new(),
                        });
                        let id = arena.len() - 1;
                        arena[current].children.push(id);
                        id
                    }
                };
                if index + 1 == chars.len() {
                    arena[next].probability = Some(*probability);
                }
                current = next;
            }
        }
        let root_children = arena[0].children.clone();
        let mut out: Vec<u8> = Vec::new();
        emit_array(&arena, &root_children, &mut out);
        out
    }

    fn push_char(out: &mut Vec<u8>, code_point: i32) {
        if code_point >= 0x20 && code_point <= 0xFF {
            out.push(code_point as u8);
        } else {
            out.push(((code_point >> 16) & 0xFF) as u8);
            out.push(((code_point >> 8) & 0xFF) as u8);
            out.push((code_point & 0xFF) as u8);
        }
    }

    fn emit_array(arena: &[TNode], nodes: &[usize], out: &mut Vec<u8>) {
        assert!(nodes.len() < 0x80, "one byte array sizes only in this helper");
        out.push(nodes.len() as u8);
        let mut child_slots: Vec<(usize, usize)> = Vec::new();
        for id in nodes {
            let node = &arena[*id];
            let has_children = !node.children.is_empty();
            let mut flags = 0u8;
            if node.probability.is_some() {
                flags |= 0x10;
            }
            if has_children {
                flags |= 0x40; // one byte children offset
            }
            out.push(flags);
            push_char(out, node.ch);
            if let Some(probability) = node.probability {
                out.push(probability);
            }
            if has_children {
                child_slots.push((out.len(), *id));
                out.push(0); // patched below
            }
        }
        // Forward link: zero, i.e. no following array.
        out.extend_from_slice(&[0, 0, 0]);
        for (field_pos, id) in child_slots {
            let start = out.len();
            let offset = start - field_pos;
            assert!(offset < 0xFF, "one byte offsets only in this helper");
            out[field_pos] = offset as u8;
            let children = arena[id].children.clone();
            emit_array(arena, &children, out);
        }
    }
}

/// A dictionary that serves the given version 2 trie from memory.
#[cfg(test)]
pub fn from_v2_trie_for_test(trie: Vec<u8>) -> Dictionary {
    let length = trie.len();
    Dictionary {
        body: DataSource::from_vec(trie),
        header: Header {
            number: 202,
            version: DictVersion::V202,
            flags: 0,
            size: 0,
            attributes: Vec::new(),
        },
        trie: Buf::new(0, length),
        terminal_table: Buf::empty(),
        language_model: Buf::empty(),
        layout: Layout::V2Static,
        code_point_table: None,
        is_v4: false,
        has_historical_info: false,
        user: None,
        corrupted: false,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_trie_reports_no_children() {
        let dict = Dictionary {
            body: DataSource::from_vec(vec![0u8; 64]),
            header: Header {
                number: 403,
                version: DictVersion::V403,
                flags: 0,
                size: 12,
                attributes: Vec::new(),
            },
            trie: Buf::new(12, 52),
            terminal_table: Buf::empty(),
            language_model: Buf::empty(),
            layout: Layout::V4,
            code_point_table: None,
            is_v4: true,
            has_historical_info: false,
            user: None,
            corrupted: false,
        };
        let root = dict.root_array();
        assert_eq!(root.count, 0);
        assert_eq!(dict.get_probability(&[0x61]), NOT_A_PROBABILITY);
    }

    #[test]
    fn writes_and_reopens_an_empty_dictionary() {
        let mut path = std::env::temp_dir();
        path.push(format!("kb-core-empty-{}.dict", std::process::id()));
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.userlog", path));

        assert!(Dictionary::write_empty_v4(
            &path,
            403,
            &[("dictionary", "user"), ("version", "403"), ("locale", "en")]
        ));

        let dict = Dictionary::open(&path, 0, 0, false).expect("empty dictionary must reopen");
        assert!(!dict.corrupted, "an empty dictionary is not a corrupt one");
        assert_eq!(dict.header.number, 403);
        assert_eq!(dict.header.locale().as_deref(), Some("en"));
        assert_eq!(dict.header.attribute_i32("version"), Some(403));
        assert_eq!(dict.root_array().count, 0);
        assert_eq!(dict.get_probability(&[0x61]), NOT_A_PROBABILITY);
        assert_eq!(dict.get_max_probability_of_exact_matches(&[0x61]), NOT_A_PROBABILITY);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_trie_round_trips_through_find_word() {
        let trie = test_trie::build(&[("hello", 200), ("help", 180), ("helm", 120)]);
        let dict = from_v2_trie_for_test(trie);
        let hello: Vec<i32> = "hello".chars().map(|c| c as i32).collect();
        let (probability, _node) = dict.find_word(&hello);
        assert_eq!(probability, 200);
        let missing: Vec<i32> = "helper".chars().map(|c| c as i32).collect();
        let (probability, _node) = dict.find_word(&missing);
        assert_eq!(probability, crate::NOT_A_TERMINAL_ID);
        assert_eq!(dict.get_probability(&hello), 200);
    }
}
