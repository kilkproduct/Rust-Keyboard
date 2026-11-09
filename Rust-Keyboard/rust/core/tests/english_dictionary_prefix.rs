//! Parses the first kilobyte of the shipped English dictionary.
//!
//! The dictionary is a version 2 Patricia trie, and the first kilobyte is
//! enough to cover the root node array and the start of the `A` subtree. The
//! expected words below were read out of the bytes by hand, so this test fails
//! as soon as the node reader drifts from the on-disk layout.

use kb_core::ptrie::{read_node, read_node_array_size, Layout, Node};

const TRIE: &[&str] = &[
    "373031576561746865721f19804101108042451a80439c7c8044f758c0450129",
    "0ec046014ce8c04701736fc04801a843d049c401dce1c04a01f0d4c04b02098a",
    "c04c022f27c04d02634ec04e02c1cfc04f02de6bc05002f693c05103360ec052",
    "033aa4c053036ac4c05403d509c055040d38c056041658c057042a55c058044f",
    "e5c05904511fc05a04588cd061d0046093c062050e03c06305af1bc06406c59a",
    "c065077c72c06607f549c067086ff6c06808ce0ed869000939c300050f491fc0",
    "6a09be8bc06b09d6b8c06c09f140c06d0a4631c06e0aeb85c06f0b338bc0700b",
    "8235c0710c76fcc0720c8526c0730d39d2c0740e841fc0750f1775c0760f9317",
    "c0770fc04dc078100630c079100809c07a100f87c0c91015d2c0e91015e81b10",
    "4d004061694062e780630442806406188065099a80660a6780670baf80680d5a",
    "80690d9f806a0ee7806b0f09806c0f81806d1b30806e1f9f806f2a7280702a8a",
    "80712cf780722dba8073365280743a7d80753d1e807640d230776c776f72741f",
    "018078420880794241807a42e903706368656e1f5107406c0a40723c01302773",
    "1f170430626f72671f4540650d7073741f351970746f1f3c1902106e27707375",
    "6e641f3101013027731f01013027731f01013027731f22043061751f3b306761",
    "751f44306875731f49706f6e1f6801033027731f455073310a7076697463681f",
    "010701306f6e1f30013027731f010e4061454062884064f69065570105a0686f",
    "7272651f01a9806901b3b06b68617a1f4601fcb06e65721f4b01ff806f020180",
    "720260807302af90751902d7807902f6307a75671f29087062611f4d3a306375",
    "7327731f013064616e1f35306761696c27731f01306b616e27731f01306e646f",
    "6e6572731f01307365721f013074746f697227731f01013027731f1205506149",
    "144065267069651f4148706f741f5b4850795554023027731f1b507358010230",
    "27731f2a3069641f4d033027731f1e6076696c6c1f0550796f12025065430730",
    "69616e1f1b013027731f01013027731f3c013027731f12023027731f3650745f",
    "01013027731f3d013027731f3802306961731f1570756c1f6501013027731f2d",
    "063027731f3730646e65676f1f26306b651f01506c5a0c306f6b7574611f3140",
    "7226043027731f3a706172641f390d3065731f2770736f6e1f3007013027731f",
    "1c013027731f010570636f6e77791f2d1f706465656e1f651d406e2f3072616e",
    "63651f1570797374777974681f4b39013027731f01023027731f357073686972",
    "651f4c01013027731f010270617468791f3e0970657468791f4007013027731f",
    "19013027731f1b013027731f0a02306e636527731f013072731f0f0470646a61",
    "6e1f41136067611f14706c656e651f4c23606e671f24013027731f0a0330656c"
];

fn trie_bytes() -> Vec<u8> {
    let hex: String = TRIE.concat();
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

fn read_array_count(data: &[u8], pos: usize) -> (usize, usize) {
    let mut read = pos;
    let count = read_node_array_size(data, &mut read);
    (count, read)
}

fn collect_words(
    data: &[u8],
    children_pos: Option<usize>,
    prefix: &str,
    out: &mut Vec<(String, i32)>,
) {
    let array_pos = match children_pos {
        Some(pos) if pos + 4 < data.len() => pos,
        _ => return,
    };
    let (count, mut pos) = read_array_count(data, array_pos);
    if count == 0 || count > 400 {
        return;
    }
    let mut index = 0;
    while index < count {
        let node: Node = match read_node(data, pos, Layout::V2Static, None) {
            Some(node) => node,
            None => return,
        };
        let mut word = String::from(prefix);
        for i in 0..node.char_count as usize {
            word.push(char::from_u32(node.chars[i] as u32).unwrap_or('?'));
        }
        if node.is_terminal() {
            out.push((word.clone(), node.probability));
        }
        if node.has_children() {
            collect_words(data, Some(node.children_pos as usize), &word, out);
        }
        if node.sibling_pos <= pos as i32 {
            return;
        }
        pos = node.sibling_pos as usize;
        index += 1;
    }
}

#[test]
fn reads_the_start_of_the_english_dictionary() {
    let data = trie_bytes();
    let mut words = Vec::new();
    // The root node array sits at position zero; `children_pos = 0` on the
    // synthetic root stands for it.
    collect_words(&data, Some(0) , "", &mut words);
    let first: Vec<&str> = words.iter().take(12).map(|(w, _)| w.as_str()).collect();
    assert_eq!(
        first,
        vec![
            "1Weather", "AM", "Aachen", "Aachen's", "Aalborg", "Aalen", "Aalesund",
            "Aalesund's", "Aalst", "Aalst's", "Aalto", "Aalto's",
        ],
        "parsed words: {:?}",
        words
    );
}
