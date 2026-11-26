//! `categories.dat`: the instance's mod categories (`CategoryFactory`, `categories.cpp`).
//!
//! One category per line, `id|name|parent` (MO2 2.5 writes this) or the older
//! `id|name|nexus ids|parent`. The Nexus mapping lives in `nexuscatmap.dat` and is not
//! touched here. Both files sit in the instance folder (`qApp->property("dataPath")`).

use std::path::{Path, PathBuf};

use crate::{Error, Result, write::write_if_changed};

/// One category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i32,
    pub name: String,
    /// 0 = top level.
    pub parent: i32,
    /// The Nexus ids cell of an old four-cell line, kept as it was.
    nexus_cell: Option<String>,
}

/// The categories of an instance, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Categories {
    path: PathBuf,
    pub list: Vec<Category>,
}

impl Categories {
    /// Reads `<instance>/categories.dat`. A missing file is an empty list (MO2 would then
    /// load its defaults on start and write them).
    pub fn open(instance_root: &Path) -> Result<Self> {
        let path = instance_root.join("categories.dat");
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(source) => return Err(Error::Read { path, source }),
        };
        Ok(Self { list: parse(&data), path })
    }

    /// The category with this id. Id 0 is "None" and is never listed.
    pub fn get(&self, id: i32) -> Option<&Category> {
        self.list.iter().find(|c| c.id == id)
    }

    /// `CategoryFactory::getCategoryID(name)`: the first category with this name (exact).
    pub fn by_name(&self, name: &str) -> Option<&Category> {
        self.list.iter().find(|c| c.name == name)
    }

    /// `CategoryFactory::addCategory(name, ...)`: the lowest free id from 1. Returns it.
    pub fn add(&mut self, name: &str, parent: i32) -> i32 {
        let id = (1..).find(|id| self.get(*id).is_none()).unwrap_or(1);
        self.list.push(Category { id, name: name.to_owned(), parent, nexus_cell: None });
        id
    }

    /// Renames a category. Returns false when the id doesn't exist.
    pub fn rename(&mut self, id: i32, name: &str) -> bool {
        match self.list.iter_mut().find(|c| c.id == id) {
            Some(c) => {
                c.name = name.to_owned();
                true
            }
            None => false,
        }
    }

    /// Removes a category; its children move to the top level. Mods that name it keep the
    /// id in their `meta.ini`; MO2 ignores ids it doesn't know.
    pub fn remove(&mut self, id: i32) -> bool {
        let before = self.list.len();
        self.list.retain(|c| c.id != id);
        for c in &mut self.list {
            if c.parent == id {
                c.parent = 0;
            }
        }
        self.list.len() != before
    }

    /// The file as `CategoryFactory::saveCategories` writes it (`\n` line ends).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for c in self.list.iter().filter(|c| c.id != 0) {
            match &c.nexus_cell {
                Some(nexus) => out.push_str(&format!("{}|{}|{nexus}|{}\n", c.id, c.name, c.parent)),
                None => out.push_str(&format!("{}|{}|{}\n", c.id, c.name, c.parent)),
            }
        }
        out.into_bytes()
    }

    /// Writes the file if it changed.
    pub fn save(&self) -> Result<bool> {
        write_if_changed(&self.path, &self.to_bytes())
    }
}

/// `CategoryFactory::loadCategories`: lines of 3 or 4 `|`-separated cells; other lines
/// are skipped (MO2 logs them). The parent cell is trimmed, the name is not.
fn parse(data: &[u8]) -> Vec<Category> {
    let mut list = Vec::new();
    for line in data.split_inclusive(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(line);
        let cells: Vec<&str> = line.split('|').collect();
        let number = |cell: &str| cell.trim().parse::<i32>().unwrap_or(0);
        let category = match cells.as_slice() {
            [id, name, parent] => Category { id: number(id), name: (*name).to_owned(), parent: number(parent), nexus_cell: None },
            [id, name, nexus, parent] => Category {
                id: number(id),
                name: (*name).to_owned(),
                parent: number(parent),
                nexus_cell: Some((*nexus).to_owned()),
            },
            _ => continue,
        };
        list.push(category);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_line_forms_and_writes_back_the_same() {
        let data = b"1|Skyrim Special Edition|0\n2|Buildings|0\n3|Old|10,11|2\nnot a line\n";
        let cats = Categories { path: PathBuf::new(), list: parse(data) };
        assert_eq!(cats.list.len(), 3);
        assert_eq!(cats.get(3).unwrap().parent, 2);
        assert_eq!(cats.by_name("Buildings").unwrap().id, 2);
        assert_eq!(cats.to_bytes(), b"1|Skyrim Special Edition|0\n2|Buildings|0\n3|Old|10,11|2\n");
    }

    #[test]
    fn add_takes_the_lowest_free_id_and_remove_lifts_children() {
        let mut cats = Categories { path: PathBuf::new(), list: parse(b"1|A|0\n3|C|1\n") };
        assert_eq!(cats.add("B", 0), 2);
        assert_eq!(cats.add("D", 0), 4);
        assert!(cats.rename(4, "Dee"));
        assert!(cats.remove(1));
        assert_eq!(cats.to_bytes(), b"3|C|0\n2|B|0\n4|Dee|0\n");
    }

    #[test]
    fn the_real_files_round_trip() {
        let mut seen = 0;
        let instances = std::env::var("STRATA_REAL_INSTANCES").unwrap_or_default();
        for instance in instances.split(':').filter(|s| !s.is_empty()) {
            let root = std::path::Path::new(instance);
            let Ok(data) = std::fs::read(root.join("categories.dat")) else { continue };
            let cats = Categories::open(&root).unwrap();
            assert!(!cats.list.is_empty());
            assert_eq!(cats.to_bytes(), data, "{instance}");
            seen += 1;
        }
        if seen == 0 {
            eprintln!("SKIPPED: the_real_files_round_trip: no categories.dat in STRATA_REAL_INSTANCES (real data, read only)");
        }
    }
}
