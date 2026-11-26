//! Prints every file of the TES4 BSAs given (folder/name, `/`-separated), one per line, as the
//! `ba2` crate reads them. A file it cannot read is reported on stderr and makes the exit code 1.
use ba2::prelude::*;

fn main() {
    let mut failed = false;
    for arg in std::env::args().skip(1) {
        let p = std::path::Path::new(&arg);
        let Ok(mut file) = std::fs::File::open(p) else {
            eprintln!("cannot open {arg}");
            failed = true;
            continue;
        };
        match ba2::guess_format(&mut file) {
            Some(ba2::FileFormat::TES4) => {
                let a = match ba2::tes4::Archive::read(p) {
                    Ok((a, _)) => a,
                    Err(e) => {
                        eprintln!("ba2 cannot read {arg}: {e:?}");
                        failed = true;
                        continue;
                    }
                };
                for (dk, d) in a.iter() {
                    for (fk, _) in d.iter() {
                        let dir = dk.name().to_string().replace('\\', "/");
                        let f = fk.name().to_string();
                        if dir == "." || dir.is_empty() { println!("{f}") } else { println!("{dir}/{f}") }
                    }
                }
            }
            _ => {
                eprintln!("not a TES4 BSA: {arg}");
                failed = true;
            }
        }
    }
    std::process::exit(i32::from(failed));
}
