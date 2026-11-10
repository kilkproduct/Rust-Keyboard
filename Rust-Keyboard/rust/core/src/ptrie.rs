//! Patricia-trie node decoding.
//!
//! Two node layouts are supported:
//!
//! * `V4` - used by every version 4 dictionary (static and dynamic).  Node:
//!   `flags, parent-offset (sint24), chars, [terminal id (u32)], children (sint24)`
//! * `V2Static` - used by version 2.02 single-buffer dictionaries.  Node:
//!   `flags, chars, [probability (u8)], children, [shortcuts], [bigrams]`
//!
//! Positions are byte offsets into the dictionary's trie buffer. Child and
//! parent addresses are relative to the position of the field itself, which is
//! why every helper tracks the exact position it read from.

use crate::bytes;

pub const FLAG_CHILDREN_TYPE_MASK: u8 = 0xC0;
pub const FLAG_IS_MOVED: u8 = 0x40;
pub const FLAG_IS_DELETED: u8 = 0x80;
pub const FLAG_HAS_MULTIPLE_CHARS: u8 = 0x20;
pub const FLAG_IS_TERMINAL: u8 = 0x10;
pub const FLAG_HAS_SHORTCUT_TARGETS: u8 = 0x08;
pub const FLAG_HAS_BIGRAMS: u8 = 0x04;
pub const FLAG_IS_NOT_A_WORD: u8 = 0x02;
pub const FLAG_IS_POSSIBLY_OFFENSIVE: u8 = 0x01;

/// `0x7FFFFF` is used by the format to mean "an offset of zero".
pub const DICT_OFFSET_ZERO_OFFSET: i32 = 0x7F_FFFF;
/// A stored offset of 0 means "no such position".
pub const DICT_OFFSET_INVALID: i32 = 0;

/// Largest character group we accept in a single node.
/// Longest character group a single node may hold. Version 4 dictionaries merge
/// a whole word into one node, so this has to be able to hold any word; it
/// matches `MAX_WORD_LENGTH`.
pub const MAX_CHARS_IN_NODE: usize = 24;
/// Largest number of bigrams we are willing to walk past when skipping a node.
const MAX_BIGRAMS_PER_NODE: usize = 10000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// Version 2.02 static dictionary.
    V2Static,
    /// Version 4 dictionary (tri, terminal table and language model separate).
    V4,
}

/// One decoded PtNode.
#[derive(Clone, Copy)]
pub struct Node {
    /// Position of the node inside the trie buffer.
    pub pos: usize,
    pub flags: u8,
    pub char_count: u8,
    pub chars: [i32; MAX_CHARS_IN_NODE],
    /// Inline probability for `V2Static`, otherwise `NOT_A_PROBABILITY`.
    pub probability: i32,
    /// Terminal id for `V4`, otherwise `NOT_A_TERMINAL_ID`.
    pub terminal_id: i32,
    pub children_pos: i32,
    /// Position just behind this node, i.e. where its next sibling starts.
    pub sibling_pos: i32,
    pub parent_pos: i32,
    pub is_moved: bool,
}

impl Node {
    pub fn empty() -> Node {
        Node {
            pos: 0,
            flags: 0,
            char_count: 0,
            chars: [0; MAX_CHARS_IN_NODE],
            probability: crate::NOT_A_PROBABILITY,
            terminal_id: crate::NOT_A_TERMINAL_ID,
            children_pos: crate::NOT_A_DICT_POS,
            sibling_pos: crate::NOT_A_DICT_POS,
            parent_pos: crate::NOT_A_DICT_POS,
            is_moved: false,
        }
    }

    #[inline]
    pub fn is_terminal(&self) -> bool {
        self.flags & FLAG_IS_TERMINAL != 0
    }

    #[inline]
    pub fn is_not_a_word(&self) -> bool {
        self.flags & FLAG_IS_NOT_A_WORD != 0
    }

    #[inline]
    pub fn is_possibly_offensive(&self) -> bool {
        self.flags & FLAG_IS_POSSIBLY_OFFENSIVE != 0
    }

    #[inline]
    pub fn has_children(&self) -> bool {
        self.children_pos != crate::NOT_A_DICT_POS
    }

    #[inline]
    pub fn char_at(&self, index: usize) -> i32 {
        if index < self.char_count as usize {
            self.chars[index]
        } else {
            crate::NOT_A_CODE_POINT
        }
    }
}

/// Reads the number of PtNodes in a PtNode array (1 or 2 bytes).
pub fn read_node_array_size(data: &[u8], pos: &mut usize) -> usize {
    let first = bytes::read_u8(data, pos);
    if first < 0x80 {
        first as usize
    } else {
        (((first & 0x7F) as usize) << 8) ^ bytes::read_u8(data, pos) as usize
    }
}

/// Reads the forward link that closes a PtNode array.
///
/// Returns the offset (relative to `link_pos`) of the next array, or 0.
pub fn read_forward_link(data: &[u8], link_pos: usize) -> i32 {
    let mut pos = link_pos;
    bytes::read_sint24(data, &mut pos)
}

/// Reads one node.
///
/// Returns `None` only for positions outside the buffer; field-level anomalies
/// (truncated groups, bad offsets) are clamped so that a corrupt dictionary
/// degrades into "no suggestions" instead of a crash.
pub fn read_node(
    data: &[u8],
    pos: usize,
    layout: Layout,
    table: Option<&[i32]>,
) -> Option<Node> {
    if pos >= data.len() {
        return None;
    }
    let mut read = pos;
    let flags = bytes::read_u8(data, &mut read);
    let mut node = Node::empty();
    node.pos = pos;
    node.flags = flags;
    node.parent_pos = crate::NOT_A_DICT_POS;

    match layout {
        Layout::V4 => {
            // The parent (or, for a moved node, the destination) offset is
            // relative to the first byte of the node itself, not to the field.
            let head_pos = pos;
            let parent_offset = bytes::read_sint24(data, &mut read);
            node.parent_pos = resolve_offset(parent_offset, head_pos);
            node.is_moved = (flags & FLAG_CHILDREN_TYPE_MASK) == FLAG_IS_MOVED;

            node.char_count = read_chars(data, flags, table, &mut read, &mut node.chars) as u8;

            if flags & FLAG_IS_TERMINAL != 0 {
                node.terminal_id = bytes::read_u32(data, &mut read) as i32;
            }

            let children_field_pos = read;
            let children_offset = bytes::read_sint24(data, &mut read);
            node.children_pos = resolve_offset(children_offset, children_field_pos);
        }
        Layout::V2Static => {
            node.char_count = read_chars(data, flags, table, &mut read, &mut node.chars) as u8;

            if flags & FLAG_IS_TERMINAL != 0 {
                node.probability = bytes::read_u8(data, &mut read) as i32;
            }

            let children_field_pos = read;
            let address_size = match flags & FLAG_CHILDREN_TYPE_MASK {
                0x40 => 1usize,
                0x80 => 2usize,
                0xC0 => 3usize,
                _ => 0usize,
            };
            if address_size == 0 {
                node.children_pos = crate::NOT_A_DICT_POS;
            } else {
                let offset = bytes::read_uint(data, address_size, &mut read) as i32;
                node.children_pos = children_field_pos as i32 + offset;
            }

            if flags & FLAG_HAS_SHORTCUT_TARGETS != 0 {
                // The 2-byte field holds the size of the whole shortcut list
                // *including* the field itself, so only the part past it has to
                // be skipped. Skipping the whole value ran two bytes into the
                // next node and turned the rest of the array into noise.
                let list_size = bytes::read_u16(data, &mut read) as usize;
                read = read.saturating_add(list_size.saturating_sub(2));
            }
            if flags & FLAG_HAS_BIGRAMS != 0 {
                read = skip_bigrams(data, read);
            }
        }
    }

    if read > data.len() {
        read = data.len();
    }
    node.sibling_pos = read as i32;
    Some(node)
}

/// Resolves a stored (relative) offset into an absolute position.
#[inline]
fn resolve_offset(offset: i32, field_pos: usize) -> i32 {
    if offset == DICT_OFFSET_INVALID {
        crate::NOT_A_DICT_POS
    } else if offset == DICT_OFFSET_ZERO_OFFSET {
        field_pos as i32
    } else {
        field_pos as i32 + offset
    }
}

fn read_chars(
    data: &[u8],
    flags: u8,
    table: Option<&[i32]>,
    read: &mut usize,
    out: &mut [i32; MAX_CHARS_IN_NODE],
) -> usize {
    if flags & FLAG_HAS_MULTIPLE_CHARS != 0 {
        bytes::read_string(data, table, MAX_CHARS_IN_NODE, out, read)
    } else {
        let cp = bytes::read_code_point(data, table, read);
        if cp == crate::NOT_A_CODE_POINT {
            0
        } else {
            out[0] = cp;
            1
        }
    }
}

/// Walks past the bigram list that follows a version 2 node.
fn skip_bigrams(data: &[u8], start: usize) -> usize {
    let mut pos = start;
    let mut count = 0usize;
    loop {
        if pos >= data.len() || count >= MAX_BIGRAMS_PER_NODE {
            return pos;
        }
        count += 1;
        let flags = bytes::read_u8(data, &mut pos);
        let address_size = match flags & 0x30 {
            0x10 => 1usize,
            0x20 => 2usize,
            0x30 => 3usize,
            _ => 0usize,
        };
        pos = pos.saturating_add(address_size);
        if flags & 0x80 == 0 {
            break;
        }
    }
    pos
}

/// Resolves a moved node (`V4` only) by following its parent link.
///
/// Returns the node that actually holds the content, or `None` when the chain
/// is broken or cyclic.
pub fn resolve_moved_node(
    data: &[u8],
    node: Node,
    layout: Layout,
    table: Option<&[i32]>,
) -> Option<Node> {
    // Where the next sibling starts is a property of the *original* node: the
    // moved node is still laid out in place, only its content lives at the
    // destination. Returning the destination's tail made the trie walk jump
    // into the middle of the next word ("Aachenr", "Aalstnd's's", ...).
    let sibling_pos = node.sibling_pos;
    let mut current = node;
    let mut hops = 0;
    while current.is_moved {
        hops += 1;
        if hops > 32 || current.parent_pos < 0 {
            return None;
        }
        current = read_node(data, current.parent_pos as usize, layout, table)?;
    }
    current.sibling_pos = sibling_pos;
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_two_byte_node_array_size() {
        let data = [0x80, 0x05];
        let mut pos = 0;
        assert_eq!(read_node_array_size(&data, &mut pos), 5);
        assert_eq!(pos, 2);
    }

    #[test]
    fn reads_single_char_terminal_v4_node() {
        // flags = terminal, no children (offset 0), parent offset 0 (invalid)
        let data = [
            0x10, // flags: terminal
            0x00, 0x00, 0x00, // parent offset: none
            0x61, // 'a'
            0x00, 0x00, 0x00, 0x07, // terminal id 7 (a V4 terminal id is 4 bytes)
            0x00, 0x00, 0x00, // children: none
        ];
        let node = read_node(&data, 0, Layout::V4, None).expect("node");
        assert!(node.is_terminal());
        assert_eq!(node.char_count, 1);
        assert_eq!(node.char_at(0), 0x61);
        assert_eq!(node.terminal_id, 7);
        assert_eq!(node.children_pos, crate::NOT_A_DICT_POS);
    }

    #[test]
    fn reads_multi_char_v4_node() {
        let mut data = Vec::new();
        data.push(FLAG_HAS_MULTIPLE_CHARS);
        data.extend_from_slice(&[0x00, 0x00, 0x00]); // parent
        data.extend_from_slice(&[b'a', b'b', bytes::CHARACTER_ARRAY_TERMINATOR]);
        data.extend_from_slice(&[0x00, 0x00, 0x00]); // no children
        let node = read_node(&data, 0, Layout::V4, None).expect("node");
        assert_eq!(node.char_count, 2);
        assert_eq!(node.char_at(0), 0x61);
        assert_eq!(node.char_at(1), 0x62);
    }

    #[test]
    fn reads_v2_terminal_probability() {
        let data = [
            0x10 | 0x40, // terminal + one-byte child offset
            0x61,        // 'a'
            0x80,        // probability 128
            0x01,        // children at field_pos + 1
        ];
        let node = read_node(&data, 0, Layout::V2Static, None).expect("node");
        assert!(node.is_terminal());
        assert_eq!(node.probability, 128);
        assert_eq!(node.children_pos, 4);
    }
}
