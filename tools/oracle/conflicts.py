#!/usr/bin/env python3
"""Reference conflicts for oracle check 3 (spec/07-oracle.md).

A second implementation of MO2's conflict rules, written from MO2's source, not from Strata's.
It prints what `strata oracle conflicts` prints; tools/oracle/check-conflicts.sh compares them.

  F <area> <folded path> <origins, lowest first, joined by |; an archived one as Mod[archive]>
  M <mod> <state> <overwrites> <overwritten by> <archive state> <archive overwrites>
    <archive overwritten by> <loose/archive state> <loose/archive overwrites>
    <loose/archive overwritten by> <hidden files: true|false>

MO2's rules (modorganizer, branch instances-root):
- Data structure (DirectoryRefresher::refresh): origin "data" = the game's Data folder at
  priority 0, then the enabled mods at priority + 1 and overwrite last. Every file is indexed,
  .mohidden and .git included; a mod's top-level Root folder is skipped
  (DirectoryEntry::onDirectoryStart).
- Root structure (OrganizerCore::rebuildRootStructure): an empty "data" placeholder, then the
  enabled mods' Root folders (not overwrite's), same priorities.
- Both drop the top-level meta.ini, readme.txt and fomod/ (DirectoryRefresher::cleanStructure).
- Strata's one difference: the Root structure leaves
  out what Root deploy never places (rootdeployer.cpp: ExcludedFolders data, data files, saves,
  and the game's inis at the top), because the overlay never shows those files either.
- Per file, origins sort by priority (FileEntry::sortOrigins); the last one wins, the rest are the
  "alternatives".
- Flags (ModInfoWithConflictInfo::doConflictCheck), per mod over both structures:
    a file with no alternatives, or whose nearest alternative is "data" (Data structure only),
    is no conflict and counts as provided; otherwise the mod is overwritten by the winner unless
    it is the winner (then it provides the file), and every other alternative goes to
    "overwrites" when the mod's priority is higher, else to "overwritten by" ("data" included).
    No files: no flag. Nothing provided: redundant. Then mixed / overwrite / overwritten.
- Hidden files: a file or a parent folder whose extension is ".mohidden".
- Archive parsing ([Settings] archive_parsing_experimental=true): after each mod's (and
  overwrite's) loose files, the files of its top-level *.bsa / *.ba2 named in the profile's
  archives.txt (exact name), archives in name order (ModInfoRegular::archives,
  DirectoryEntry::addFromAllBSAs); a mod already on a path keeps its first entry. Archived entries
  sort before loose ones, by their plugin's load order (-1 last). In doConflictCheck an archived
  side sends the pair to the archive lists: archive vs archive by order, loose vs archive to the
  "loose/archive" lists.

Names in the output: "data" is "<game>", Overwrite is "<overwrite>" (Strata's layer ids).
Reads only.

usage: conflicts.py <instance folder> [profile]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from merge import ROOT_EXCLUDE, enabled_mods, find_dir, host_path, ini_value  # noqa: E402

GAME, OVERWRITE = "<game>", "<overwrite>"


def walk(folder, skip_top_root, top_exclude=()):
    """Yields (relative path parts) of every file under folder, MO2's walker style, leaving out
    the top-level names in top_exclude (lower case)."""
    stack = [(folder, [])]
    while stack:
        real, parts = stack.pop()
        try:
            entries = list(os.scandir(real))
        except OSError:
            continue
        for e in entries:
            try:
                e.name.encode("utf-8")
            except UnicodeEncodeError:
                continue  # Strata leaves names that are not UTF-8 out
            if not parts and e.name.lower() in top_exclude:
                continue
            if e.is_dir():
                if skip_top_root and not parts and e.name.lower() == "root":
                    continue
                stack.append((e.path, parts + [e.name]))
            elif e.is_file():
                yield parts + [e.name]


def cleaned(parts):
    """DirectoryRefresher::cleanStructure."""
    if len(parts) == 1 and parts[0].lower() in ("meta.ini", "readme.txt"):
        return False
    if len(parts) > 1 and parts[0].lower() == "fomod":
        return False
    return True


def hidden(parts):
    return any(os.path.splitext(p)[1] == ".mohidden" for p in parts)


def bsa_names(path):
    """The file paths of a BSA (v103-105) or BA2, from the format (UESP's pages), independent of
    Strata's reader. None when the file is not one."""
    import struct
    with open(path, "rb") as f:
        magic = f.read(4)
        if magic == b"BSA\0":
            version, offset, flags, folders, files, _, names_len, _ = struct.unpack("<IIIIIIII", f.read(32))
            f.seek(offset)
            counts = []
            for _ in range(folders):
                if version == 105:
                    _, count, _, _ = struct.unpack("<QIIQ", f.read(24))
                else:
                    _, count, _ = struct.unpack("<QII", f.read(16))
                counts.append(count)
            dirs = []
            for count in counts:
                name = ""
                if flags & 1:
                    length = f.read(1)[0]
                    name = f.read(length).split(b"\0")[0].decode("utf-8", "replace")
                f.read(16 * count)
                dirs += [name] * count
            names = f.read(names_len).split(b"\0") if flags & 2 else [b""] * files
            out = []
            for d, n in zip(dirs, names):
                d = d.replace("\\", "/").strip("/")
                n = n.decode("utf-8", "replace")
                out.append(n if d in ("", ".") else f"{d}/{n}")
            return out
        if magic == b"BTDX":
            version, kind, count, names_at = struct.unpack("<I4sIQ", f.read(20))
            f.seek(names_at)
            out = []
            for _ in range(count):
                length = struct.unpack("<H", f.read(2))[0]
                out.append(f.read(length).decode("utf-8", "replace").replace("\\", "/"))
            return out
    return None


def archive_order(name, load_order):
    """DirectoryEntry::addFromAllBSAs: the load-order index of the plugin whose stem starts the
    archive name ("<stem> - " or "<stem>."), the last match winning; -1 when none."""
    lower = name.lower()
    order = -1
    for plugin in load_order:
        stem = os.path.splitext(plugin)[0].lower()
        if lower.startswith(stem + " - ") or lower.startswith(stem + "."):
            order = load_order.index(plugin)
    return order


class Structure:
    """MO2's DirectoryEntry, reduced to what conflicts need: per folded path, every origin with
    (priority, archive name or None, archive order). An origin is added to a file once; later adds
    of the same origin are ignored (FileEntry::addOrigin)."""

    def __init__(self, base):
        self.base = base           # origins that are the game's own (baseIDs)
        self.files = {}            # folded path -> {origin: (priority, archive, order)}
        self.by_origin = {}        # origin -> [folded paths]
        self.hidden = set()        # origins with hidden files

    def put(self, origin, priority, parts, archive=None, order=-1):
        if not cleaned(parts):
            return
        key = "/".join(p.lower() for p in parts)
        origins = self.files.setdefault(key, {})
        if origin not in origins:
            origins[origin] = (priority, archive, order)
            self.by_origin.setdefault(origin, []).append(key)
        if hidden(parts):
            self.hidden.add(origin)

    def add(self, origin, priority, folder, skip_top_root, top_exclude=()):
        for parts in walk(folder, skip_top_root, top_exclude):
            self.put(origin, priority, parts)

    def add_archives(self, origin, priority, folder, enabled, load_order):
        try:
            names = [e.name for e in os.scandir(folder) if e.is_file()]
        except OSError:
            return
        wanted = [n for n in names if os.path.splitext(n)[1].lower() in (".bsa", ".ba2") and n in enabled]
        for name in sorted(wanted, key=str.lower):
            try:
                paths = bsa_names(os.path.join(folder, name))
            except (OSError, ValueError, IndexError, Exception):  # noqa: B014 - MO2 skips bad archives
                paths = None
            if paths is None:
                continue
            order = archive_order(name, load_order)
            for path in paths:
                self.put(origin, priority, path.split("/"), name, order)

    def sorted_origins(self, key):
        """FileEntry::sortOrigins: archived entries first by order (-1 last), then loose ones by
        priority; archives with the same order by priority."""
        o = self.files[key]
        big = 2 ** 31 - 1

        def sort_key(name):
            priority, archive, order = o[name]
            if archive is None:
                return (1, 0, priority)
            return (0, big if order < 0 else order, priority)
        return sorted(o, key=sort_key)


def state_of(over, under):
    if over and under:
        return "mixed"
    if over:
        return "overwrite"
    if under:
        return "overwritten"
    return "none"


def check(mod, priority, structures):
    """doConflictCheck for one mod: the three states, their lists, hidden files."""
    lists = {k: {} for k in ("ow", "owd", "a_ow", "a_owd", "al_ow", "al_owd")}
    provides = False
    count = 0
    has_hidden = False
    big = 2 ** 31 - 1
    for ds in structures:
        files = ds.by_origin.get(mod, [])
        count += len(files)
        has_hidden |= mod in ds.hidden
        for key in files:
            info = ds.files[key]
            order = ds.sorted_origins(key)
            winner, alternatives = order[-1], order[:-1]
            if not alternatives or alternatives[-1] in ds.base:
                provides = True
                continue
            mine = info[mod][1] is not None
            my_order = info[mod][2] if mine else None
            if my_order is not None and my_order < 0:
                my_order = big
            if winner != mod:
                if info[winner][1] is not None:
                    lists["a_owd"][winner] = info[winner][0]
                elif mine:
                    lists["al_owd"][winner] = info[winner][0]
                else:
                    lists["owd"][winner] = info[winner][0]
            else:
                provides = True
            for alt in alternatives:
                if alt == mod:
                    continue
                alt_priority, alt_archive, alt_order = info[alt]
                if alt_order < 0:
                    alt_order = big
                if alt_archive is None and not mine:
                    lists["ow" if priority > alt_priority else "owd"][alt] = alt_priority
                elif alt_archive is None:
                    lists["al_owd"][alt] = alt_priority
                elif not mine:
                    lists["al_ow"][alt] = alt_priority
                elif my_order > alt_order:
                    lists["a_ow"][alt] = alt_priority
                elif my_order < alt_order:
                    lists["a_owd"][alt] = alt_priority
    by_priority = {k: [n for n, _ in sorted(d.items(), key=lambda kv: kv[1])] for k, d in lists.items()}
    if count == 0:
        return ["none", [], [], "none", [], [], "none", [], [], False]
    state = "redundant" if not provides else state_of(lists["ow"], lists["owd"])
    return [state, by_priority["ow"], by_priority["owd"],
            state_of(lists["a_ow"], lists["a_owd"]), by_priority["a_ow"], by_priority["a_owd"],
            state_of(lists["al_ow"], lists["al_owd"]), by_priority["al_ow"], by_priority["al_owd"],
            has_hidden]


def read_lines(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return [line.strip() for line in f if line.strip()]
    except FileNotFoundError:
        return []


def main():
    instance = os.path.abspath(sys.argv[1])
    with open(os.path.join(instance, "ModOrganizer.ini"), encoding="utf-8", errors="replace") as f:
        ini = f.read()
    profile = sys.argv[2] if len(sys.argv) > 2 else ini_value(ini, "General", "selected_profile")
    game = host_path(ini_value(ini, "General", "gamePath"))

    data = Structure(base={GAME})
    root = Structure(base=set())
    game_data = find_dir(game, "Data")
    if game_data:
        data.add(GAME, 0, game_data, skip_top_root=False)
    mods = enabled_mods(instance, profile)
    priorities = {}
    for i, name in enumerate(mods):
        priorities[name] = i + 1
        folder = os.path.join(instance, "mods", name)
        data.add(name, i + 1, folder, skip_top_root=True)
        root_folder = find_dir(folder, "Root")
        if root_folder:
            root.add(name, i + 1, root_folder, skip_top_root=False, top_exclude=ROOT_EXCLUDE)
    priorities[OVERWRITE] = len(mods) + 1
    data.add(OVERWRITE, len(mods) + 1, os.path.join(instance, "overwrite"), skip_top_root=True)

    # MO2's archive parsing: after each mod's loose files (ModThread::run), its loaded archives.
    if (ini_value(ini, "Settings", "archive_parsing_experimental") or "").lower() == "true":
        profile_dir = os.path.join(instance, "profiles", profile)
        enabled = set(read_lines(os.path.join(profile_dir, "archives.txt")))
        load_order = [l.lstrip("*") for l in read_lines(os.path.join(profile_dir, "loadorder.txt"))
                      if not l.startswith("#")]
        for name in mods + [OVERWRITE]:
            folder = os.path.join(instance, "overwrite" if name == OVERWRITE else os.path.join("mods", name))
            data.add_archives(name, priorities[name], folder, enabled, load_order)

    out = []
    for area, ds in (("data", data), ("root", root)):
        for key in ds.files:
            shown = [n if ds.files[key][n][1] is None else f"{n}[{ds.files[key][n][1]}]" for n in ds.sorted_origins(key)]
            out.append(f"F\t{area}\t{key}\t{'|'.join(shown)}")
    for name in mods + [OVERWRITE]:
        r = check(name, priorities[name], (data, root))
        cells = [x if isinstance(x, str) else ("true" if x is True else "false" if x is False else "|".join(x)) for x in r]
        out.append("M\t" + name + "\t" + "\t".join(cells))
    sys.stdout.write("\n".join(out) + ("\n" if out else ""))


if __name__ == "__main__":
    main()
