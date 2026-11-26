#!/usr/bin/env python3
"""Reference merged tree for oracle check 2a (spec/07-oracle.md).

A second, independent implementation of the overlay's rules, written from the spec and
MO2's behaviour, small enough to read in one sitting. It prints what `strata tree --origin`
prints: one line per file, `path<TAB>origin`, where origin is the mod's name, `<game>` or
`<overwrite>`. tools/oracle/check-tree.sh compares the two.

Rules (spec/03-vfs.md):
- Layers, lowest first: the game folder, enabled mods (modlist.txt read bottom to top),
  overwrite. The highest layer that has a path wins. Folders merge.
- Names compare without regard to case; the first spelling seen (lowest layer) is shown.
- A mod's files appear under Data/, except its top-level Root folder and meta.ini.
  Root's content appears in the game folder itself, except Data, Data Files, Saves and the
  game's inis.
- *.mohidden files and .git folders are never shown.
- Paths listed in <instance>/strata/whiteouts.txt are hidden, unless overwrite has them.
- Separators (<name>_separator) and backups (...backup<n>) are not layers.

usage: merge.py <instance folder> [profile]
"""
import os
import re
import sys

INIS = {"skyrim.ini", "skyrimprefs.ini", "skyrimcustom.ini"}
ROOT_EXCLUDE = {"data", "data files", "saves"} | INIS
# The game's own inis by gameName (game_skyrimse / game_skyrim `iniFiles`).
GAME_INIS = {
    "skyrim special edition": INIS,
    "skyrim": {"skyrim.ini", "skyrimprefs.ini"},
    "oblivion": {"oblivion.ini", "oblivionprefs.ini"},
}


def root_exclude(game_name):
    """What a Root folder never places, for the instance's game."""
    return {"data", "data files", "saves"} | GAME_INIS.get((game_name or "").lower(), INIS)
MOD_EXCLUDE = {"root", "meta.ini"}


def ini_value(text, section, key):
    """The raw value of key in [section]; enough of QSettings for the keys read here."""
    current = "General"
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("["):
            current = line[1:line.index("]")]
        elif "=" in line and current == section:
            k, v = line.split("=", 1)
            if k.strip() == key:
                m = re.fullmatch(r"@ByteArray\((.*)\)", v.strip())
                return (m.group(1) if m else v.strip()).replace("\\\\", "\\")
    return None


def host_path(stored):
    if re.match(r"^[Zz]:", stored):
        return "/" + stored[2:].replace("\\", "/").lstrip("/")
    return stored.replace("\\", "/")


def enabled_mods(instance, profile):
    """Enabled regular mods, lowest priority first."""
    on_disk = {name.lower(): name for name in os.listdir(os.path.join(instance, "mods"))
               if os.path.isdir(os.path.join(instance, "mods", name))}
    seen, result = set(), []
    with open(os.path.join(instance, "profiles", profile, "modlist.txt"), "rb") as f:
        lines = [line.strip().decode("utf-8", "replace") for line in f]
    for line in lines:
        if not line or line[0] in "#*-":
            continue
        name = (line[1:] if line[0] == "+" else line).strip()
        if not name or name in seen:
            continue
        seen.add(name)
        real = on_disk.get(name.lower())
        if real is None or real.endswith("_separator") or re.fullmatch(r".*backup[0-9]*", real):
            continue
        result.append(real)
    result.reverse()  # the top line is the highest priority
    return result


def walk(real, target, exclude):
    """Yields (mount path parts, real file path) for every file under `real`."""
    try:
        entries = sorted(os.scandir(real), key=lambda e: e.name)
    except OSError:
        return
    for entry in entries:
        low = entry.name.lower()
        if low in exclude:
            continue
        if entry.is_dir():
            if low != ".git":
                yield target + [entry.name], None  # the folder itself
                yield from walk(entry.path, target + [entry.name], set())
        elif entry.is_file() and not low.endswith(".mohidden"):
            yield target + [entry.name], entry.path


def find_dir(parent, name):
    try:
        for entry in os.scandir(parent):
            if entry.name.lower() == name.lower() and entry.is_dir():
                return entry.path
    except OSError:
        pass
    return None


class Tree:
    def __init__(self):
        self.files = {}      # lower-case path -> origin
        self.names = {}      # lower-case path -> spelling shown (files and folders)
        self.dirs = set()    # lower-case paths that are folders

    def shown(self, parts):
        """The path as shown: each part in the first spelling seen."""
        out = []
        for i in range(len(parts)):
            key = "/".join(p.lower() for p in parts[:i + 1])
            out.append(self.names.setdefault(key, parts[i]))
        return "/".join(out)

    def add(self, parts, origin, is_file):
        key = "/".join(p.lower() for p in parts)
        self.shown(parts)
        prefix = key + "/"
        if is_file:
            if key in self.dirs:
                # a file replaces a folder of the same name, with everything in it
                for k in [k for k in self.files if k.startswith(prefix)]:
                    del self.files[k]
                self.dirs = {d for d in self.dirs if d != key and not d.startswith(prefix)}
            self.files[key] = origin
        else:
            self.files.pop(key, None)  # a folder replaces a file of the same name
            self.dirs.add(key)

    def add_layer(self, source, origin, rules):
        for sub, target, exclude in rules:
            real = source if not sub else find_dir(source, sub)
            if real:
                for parts, path in walk(real, target, exclude):
                    self.add(parts, origin, path is not None)

    def hide(self, path):
        key = path.lower().strip("/")
        prefix = key + "/"
        for k in [k for k in self.files if k == key or k.startswith(prefix)]:
            del self.files[k]


def main():
    instance = os.path.abspath(sys.argv[1])
    with open(os.path.join(instance, "ModOrganizer.ini"), encoding="utf-8", errors="replace") as f:
        ini = f.read()
    profile = sys.argv[2] if len(sys.argv) > 2 else ini_value(ini, "General", "selected_profile")
    game = host_path(ini_value(ini, "General", "gamePath"))
    mod_rules = [("", ["Data"], MOD_EXCLUDE), ("Root", [], root_exclude(ini_value(ini, "General", "gameName")))]

    tree = Tree()
    tree.add_layer(game, "<game>", [("", [], set())])
    for name in enabled_mods(instance, profile):
        tree.add_layer(os.path.join(instance, "mods", name), name, mod_rules)
    try:
        with open(os.path.join(instance, "strata", "whiteouts.txt"), encoding="utf-8") as f:
            for line in f:
                if line.strip() and not line.startswith("#"):
                    tree.hide(line.strip())
    except FileNotFoundError:
        pass
    tree.add_layer(os.path.join(instance, "overwrite"), "<overwrite>", mod_rules)

    out = []
    for key, origin in tree.files.items():
        parts = key.split("/")
        shown = "/".join(tree.names["/".join(parts[:i + 1])] for i in range(len(parts)))
        out.append(f"{shown}\t{origin}")
    sys.stdout.write("\n".join(sorted(out)) + ("\n" if out else ""))


if __name__ == "__main__":
    main()
