//! Round-trip check of real ini files: `ini_roundtrip <file>...` (or file names on stdin, one per line).
//!
//! For each file: (1) parse + write must give the same bytes, (2) every key and value must
//! re-encode to exactly what the file holds (so that a value Strata writes looks like Qt's).
//! Prints one line per mismatch and a summary; exits 1 when (1) fails anywhere.

use std::io::BufRead;

use strata_ini::{Document, codec};

fn main() {
    let mut files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        files = std::io::stdin().lock().lines().map_while(Result::ok).collect();
    }
    let (mut identical, mut different, mut unreadable) = (0u64, 0u64, 0u64);
    let (mut entries, mut key_mismatch, mut value_mismatch) = (0u64, 0u64, 0u64);
    for file in &files {
        let Ok(data) = std::fs::read(file) else {
            unreadable += 1;
            println!("UNREADABLE\t{file}");
            continue;
        };
        let doc = Document::parse(&data);
        if doc.to_bytes() == data {
            identical += 1;
        } else {
            different += 1;
            println!("BYTES DIFFER\t{file}");
        }
        for entry in doc.entries() {
            entries += 1;
            if codec::escape_key(entry.key) != entry.raw_key {
                key_mismatch += 1;
                println!("KEY\t{file}\t{}", String::from_utf8_lossy(entry.raw_key));
            }
            let raw = entry.raw_value;
            if codec::encode_value(&entry.value()) != raw {
                value_mismatch += 1;
                println!("VALUE\t{file}\t{}={}", entry.key, String::from_utf8_lossy(&raw[..raw.len().min(200)]));
            }
        }
    }
    println!(
        "files {} identical {identical} different {different} unreadable {unreadable} | entries {entries} \
         key-mismatch {key_mismatch} value-mismatch {value_mismatch}",
        files.len()
    );
    if different > 0 || unreadable > 0 {
        std::process::exit(1);
    }
}
