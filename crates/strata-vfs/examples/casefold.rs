//! Case folding: Strata's `fold()` against what Windows does.
//!
//!   cargo run -p strata-vfs --example casefold -- <l_intl.nls> [folder...]
//!
//! `l_intl.nls` is the case table Windows and Wine compare names with (Wine's
//! `RtlUpcaseUnicodeChar`; a Proton has one in `files/share/wine/nls/`). NTFS keeps its own
//! copy of such a table (`$UpCase`), written when the volume was formatted.
//!
//! Prints the BMP characters whose matching differs between `fold()` and the table, then
//! walks the folders (read only) and lists every name that holds a character one rule
//! folds and the other leaves alone, and every group of names in one folder that one rule
//! takes as one name and the other not (a collision or a miss).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use strata_vfs::tree::fold;

/// Windows' upper case of one UTF-16 unit, from the table's three levels (Wine's `casemap`).
fn upcase(table: &[u16], ch: u16) -> u16 {
    let first = table[usize::from(ch >> 8)];
    let second = table[usize::from(first) + usize::from((ch >> 4) & 0x0f)];
    ch.wrapping_add(table[usize::from(second) + usize::from(ch & 0x0f)])
}

/// A name as Windows compares it: every UTF-16 unit upper-cased through the table.
fn windows_key(table: &[u16], name: &str) -> Vec<u16> {
    name.encode_utf16().map(|u| upcase(table, u)).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(nls) = args.first() else {
        eprintln!("usage: casefold <l_intl.nls> [folder...]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(nls).expect("cannot read the table");
    let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    // two header words, then the upper-case table
    let table = &words[2..];
    assert_eq!(upcase(table, u16::from(b'a')), u16::from(b'A'), "not a case table");

    // Characters (BMP, no surrogates) grouped by each rule.
    let chars: Vec<char> = (0u32..0x1_0000).filter_map(char::from_u32).collect();
    let mut by_windows: BTreeMap<u16, BTreeSet<char>> = BTreeMap::new();
    let mut by_fold: BTreeMap<String, BTreeSet<char>> = BTreeMap::new();
    for c in &chars {
        let mut buf = [0u16; 2];
        by_windows.entry(upcase(table, c.encode_utf16(&mut buf)[0])).or_default().insert(*c);
        by_fold.entry(fold(&c.to_string())).or_default().insert(*c);
    }
    let windows_group = |c: char| {
        let mut buf = [0u16; 2];
        &by_windows[&upcase(table, c.encode_utf16(&mut buf)[0])]
    };
    let mut differ = BTreeSet::new();
    println!("BMP characters that match differently (fold() group | Windows group):");
    for c in &chars {
        let ours = &by_fold[&fold(&c.to_string())];
        let theirs = windows_group(*c);
        if ours != theirs && differ.insert(*c) {
            let show = |set: &BTreeSet<char>| set.iter().map(|c| format!("{c} U+{:04X}", u32::from(*c))).collect::<Vec<_>>().join(", ");
            println!("  {} U+{:04X}: {} | {}", c, u32::from(*c), show(ours), show(theirs));
        }
    }
    println!("{} characters differ", differ.len());
    // The ones that matter in a name: one rule folds them and the other leaves them alone
    // (`k` differs only because fold() also folds the Kelvin sign to it).
    let rare: BTreeSet<char> = differ
        .iter()
        .copied()
        .filter(|c| windows_group(*c).len() == 1 || by_fold[&fold(&c.to_string())].len() == 1)
        .collect();
    println!("in names they matter as: {}", rare.iter().map(|c| format!("{c} U+{:04X}", u32::from(*c))).collect::<Vec<_>>().join(", "));

    for folder in &args[1..] {
        let mut names = 0usize;
        let mut odd: Vec<String> = Vec::new();
        let mut clashes: Vec<String> = Vec::new();
        let mut stack = vec![Path::new(folder).to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut here: Vec<String> = Vec::new();
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else { continue };
                names += 1;
                if name.chars().any(|c| rare.contains(&c)) {
                    odd.push(entry.path().display().to_string());
                }
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(entry.path());
                }
                here.push(name);
            }
            // The two rules group the names; any group that is not a group of the other rule
            // holds a pair one rule takes as the same name and the other not.
            let mut by_fold: BTreeMap<String, BTreeSet<&String>> = BTreeMap::new();
            let mut by_windows: BTreeMap<Vec<u16>, BTreeSet<&String>> = BTreeMap::new();
            for name in &here {
                by_fold.entry(fold(name)).or_default().insert(name);
                by_windows.entry(windows_key(table, name)).or_default().insert(name);
            }
            for group in by_fold.values() {
                let first = *group.iter().next().expect("a group has a name");
                if group != &by_windows[&windows_key(table, first)] {
                    clashes.push(format!("{}: fold() takes {group:?} as one name, Windows does not", dir.display()));
                }
            }
            for group in by_windows.values() {
                let first = *group.iter().next().expect("a group has a name");
                if group != &by_fold[&fold(first)] {
                    clashes.push(format!("{}: Windows takes {group:?} as one name, fold() does not", dir.display()));
                }
            }
        }
        println!("{folder}: {names} names, {} with a character that differs, {} groups that differ", odd.len(), clashes.len());
        for line in odd.iter().chain(&clashes) {
            println!("  {line}");
        }
    }
}
