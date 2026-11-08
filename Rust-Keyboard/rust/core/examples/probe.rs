//! Dictionary probe.
//!
//! Small command line tool used to validate the Rust reader against real
//! dictionaries without going through Android:
//!
//! ```text
//! cargo run --release --example probe -- /data/.../en_wordlist.combined.dict hello help
//! ```
//!
//! It prints the header, maps the file read-only (the whole point of the Rust
//! engine: no heap copy), resolves every word you pass, and reports whether the
//! bytes were served from the page cache or from a private copy.

use std::env;

use kb_core::dict::Dictionary;

fn main() {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() < 2 {
        eprintln!("usage: probe <dictionary-file-or-directory> [word ...]");
        std::process::exit(2);
    }
    let path = &arguments[1];
    let dictionary = match Dictionary::open(path, 0, 0, false) {
        Some(value) => value,
        None => {
            eprintln!("could not open {} as a dictionary", path);
            std::process::exit(1);
        }
    };

    println!("path            : {}", path);
    println!("format version  : {}", dictionary.format_version());
    println!("header size     : {} bytes", dictionary.header_size());
    println!("corrupted       : {}", dictionary.corrupted);
    println!(
        "locale          : {}",
        dictionary.header().locale().unwrap_or_else(|| "?".to_string())
    );
    println!("attributes      : {}", dictionary.header().attributes.len());
    let root = dictionary.root_array();
    println!("root node count : {}", root.count);

    let mut words = 0usize;
    let mut budget = 200_000usize;
    let started = std::time::Instant::now();
    dictionary.for_each_word(|_word, _terminal, _probability| {
        words += 1;
        budget = budget.saturating_sub(1);
        budget > 0
    });
    println!(
        "walked words    : {} ({} ms)",
        words,
        started.elapsed().as_millis()
    );

    for word in arguments.iter().skip(2) {
        let code_points: Vec<i32> = word.chars().map(|c| c as i32).collect();
        let probability = dictionary.get_probability(&code_points);
        let max_probability = dictionary.get_max_probability_of_exact_matches(&code_points);
        println!(
            "lookup {:<16} probability={:<4} maxPrefixProbability={:<4} contained={}",
            word,
            probability,
            max_probability,
            dictionary.contains(&code_points)
        );
    }

    if dictionary.corrupted {
        std::process::exit(3);
    }
}
