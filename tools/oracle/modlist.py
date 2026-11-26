#!/usr/bin/env python3
"""Mod list oracle: MO2's mod list rules, written straight from the MO2 2.5.2 source, and a
random-operations check against Strata (the modlist property of spec/07-oracle.md).

usage: modlist.py check <instance> [--profile P] [--steps N] [--seed S] [--strata EXE]

The instance must be a disposable copy: Strata writes its modlist.txt. Every step runs one
operation through the Strata CLI and the same operation in this model, then checks:
  1. what Strata lists  == what this model reads from the files on disk, and
  2. what this model reads == the model's own state after the operation.
Exits 1 on the first mismatch.

Sources (modorganizer/src): Profile::refreshModStatus (reading), Profile::doWriteModlist
(writing), Profile::setModsEnabled, Profile::setModPriority (note: it clamps the caller's
priority in place, `int& newPriority`), ModList::changeModPriority, ModList::shiftModsPriority,
ModInfo::isSeparatorName / isBackupName, ModInfo::ByName, GamebryoUnmangedMods::mods.
"""
import argparse
import json
import os
import random
import re
import subprocess
import sys

BASE_PLUGINS = ["skyrim.esm", "update.esm", "dawnguard.esm", "hearthfires.esm", "dragonborn.esm"]


def ini_value(text, section, key):
    """A plain value from a QSettings ini (enough for gamePath and display_foreign)."""
    current = None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("[") and line.endswith("]"):
            current = line[1:-1]
        elif current == section and "=" in line:
            k, v = line.split("=", 1)
            if k.strip() == key:
                return v.strip()
    return None


def game_path(instance):
    text = open(os.path.join(instance, "ModOrganizer.ini"), encoding="utf-8", errors="replace").read()
    raw = ini_value(text, "General", "gamePath") or ""
    m = re.fullmatch(r"@ByteArray\((.*)\)", raw)
    if m:
        raw = m.group(1)
    raw = raw.replace("\\\\", "\\")
    if raw[:2].upper() == "Z:":
        return raw[2:].replace("\\", "/"), text
    return raw.replace("\\", "/"), text


def kind_of(name):
    if re.fullmatch(r".*backup[0-9]*", name, re.S):  # createFrom checks backups first
        return "backup"
    if re.fullmatch(r".*_separator", name, re.S):
        return "separator"
    return "mod"


def foreign_mods(instance):
    """GamebryoUnmangedMods::mods(!displayForeign) for Skyrim SE."""
    game, ini = game_path(instance)
    display = ini_value(ini, "Settings", "display_foreign")
    if display is not None and display.lower() in ("false", "0", ""):
        return []
    primary = list(BASE_PLUGINS)
    try:
        for line in open(os.path.join(game, "Skyrim.ccc"), encoding="utf-8", errors="replace"):
            if line.strip() and line.strip().lower() not in primary:
                primary.append(line.strip().lower())
    except OSError:
        pass
    data = os.path.join(game, "Data")
    names = []
    try:
        for f in os.listdir(data):
            if os.path.isfile(os.path.join(data, f)) and f.lower()[-4:] in (".esp", ".esm", ".esl") and f.lower() not in primary:
                names.append("Unmanaged: " + f[:-4])
    except OSError:
        pass
    return names


def read_listed(path):
    """refreshModStatus's loop over modlist.txt: (name, enabled) in file order, a name seen
    before (exactly) skipped."""
    out, seen = [], set()
    with open(path, "rb") as f:
        for raw in f.read().split(b"\n"):
            line = raw.strip()
            if not line or line[:1] == b"#":
                continue
            enabled = True
            if line[:1] == b"-":
                enabled, name = False, line[1:].strip()
            elif line[:1] in (b"+", b"*"):
                name = line[1:].strip()
            else:
                name = line
            name = name.decode("utf-8", "replace")
            if not name or name in seen:
                continue
            seen.add(name)
            out.append((name, enabled))
    return out


def read(instance, profile):
    """Profile::refreshModStatus: [(name, kind, enabled)] lowest priority first."""
    mods_dir = os.path.join(instance, "mods")
    collection = [(n, kind_of(n)) for n in os.listdir(mods_dir) if os.path.isdir(os.path.join(mods_dir, n))]
    collection += [(n, "foreign") for n in foreign_mods(instance)]
    collection.sort(key=lambda m: m[0].lower())  # ModInfo::ByName
    by_name = {}
    for i, (n, _) in enumerate(collection):
        by_name.setdefault(n.lower(), i)  # FileNameComparator: case-insensitive

    status = [{"priority": -1, "enabled": False} for _ in collection]
    index = 0
    for name, enabled in read_listed(os.path.join(instance, "profiles", profile, "modlist.txt")):
        i = by_name.get(name.lower())
        if i is None or collection[i][1] == "backup":  # hasAutomaticPriority: skipped
            continue
        status[i]["enabled"] = enabled
        if status[i]["priority"] == -1:
            status[i]["priority"] = index
            index += 1
    known = index
    top = 0
    regular = 0
    for i, (name, kind) in enumerate(collection):
        if kind == "foreign":
            status[i]["enabled"] = True  # alwaysEnabled
        if status[i]["priority"] != -1:
            status[i]["priority"] = known - status[i]["priority"] - 1
            regular += 1
        elif kind == "foreign":
            top -= 1
            status[i]["priority"] = top
            regular += 1
        elif kind != "backup":
            status[i]["priority"] = index
            index += 1
            regular += 1
    if top < 0:
        for i, (_, kind) in enumerate(collection):
            if kind != "backup":
                status[i]["priority"] -= top
    backup = regular
    for i, (_, kind) in enumerate(collection):
        if kind == "backup":
            status[i]["priority"] = backup
            status[i]["enabled"] = False  # alwaysDisabled (never enabled by the list)
            backup += 1
    order = sorted(range(len(collection)), key=lambda i: status[i]["priority"])
    return [(collection[i][0], collection[i][1], status[i]["enabled"]) for i in order]


class Model:
    """The profile's mod status after reading, and MO2's operations on it."""

    def __init__(self, entries):
        self.entries = [list(e) for e in entries]  # [name, kind, enabled], by priority
        self.regular = sum(1 for e in entries if e[1] != "backup")

    def find(self, name):
        for i, e in enumerate(self.entries):
            if e[0] == name:
                return i
        for i, e in enumerate(self.entries):
            if e[0].lower() == name.lower():
                return i
        return None

    def set_enabled(self, names, enabled):
        """Profile::setModsEnabled."""
        for n in names:
            e = self.entries[self.find(n)]
            if enabled and e[1] == "backup":
                continue
            if not enabled and e[1] == "foreign":
                continue
            e[2] = enabled

    def set_priority(self, index_name, new_priority):
        """Profile::setModPriority on priorities, as written in MO2 (the loop). Returns the
        clamped priority, which MO2 writes back into the caller's variable."""
        old = self.find(index_name)
        if self.entries[old][1] == "backup":
            return new_priority
        new = max(0, min(new_priority, self.regular - 1))
        if new == old:
            return new
        prio = {e[0]: p for p, e in enumerate(self.entries)}
        last = len(self.entries) - 1
        for name, p in list(prio.items()):
            if new < old and new <= p < old:
                prio[name] = p + 1
            elif new > old and old < p <= new:
                prio[name] = p - 1
        prio[self.entries[old][0]] = min(new, last)
        self.entries.sort(key=lambda e: prio[e[0]])
        return min(new, last)

    def change_priority(self, names, new_priority):
        """ModList::changeModPriority (several mods) / changeModsPriority (one mod)."""
        names = [self.entries[self.find(n)][0] for n in names]
        names = list(dict.fromkeys(names))
        if len(names) == 1:
            self.set_priority(names[0], new_priority)
            return
        pos = lambda n: self.find(n)
        for n in sorted(names, key=pos, reverse=True):
            if pos(n) > new_priority:
                new_priority = self.set_priority(n, new_priority)
        ordered = sorted(names, key=pos)
        for n in ordered:
            if pos(n) < new_priority:
                new_priority -= 1
                break
        for n in ordered:
            if pos(n) < new_priority:
                new_priority = self.set_priority(n, new_priority)

    def shift(self, names, offset):
        """ModList::shiftModsPriority."""
        names = [self.entries[self.find(n)][0] for n in names]
        names.sort(key=self.find, reverse=offset > 0)
        for n in names:
            self.set_priority(n, max(self.find(n) + offset, 0))

    def as_tuples(self):
        return [tuple(e) for e in self.entries]


def strata_list(strata, instance, profile):
    out = subprocess.run([strata, "list", "--json", "-i", instance, "-p", profile], check=True, capture_output=True, text=True).stdout
    kinds = {"mod": "mod", "separator": "separator", "foreign": "foreign", "backup": "backup"}
    return [(m["name"], kinds[m["kind"]], m["enabled"]) for m in json.loads(out)["mods"]]


def check(args):
    strata = args.strata
    profile = args.profile
    if profile is None:
        _, ini = game_path(args.instance)
        raw = ini_value(ini, "General", "selected_profile") or "Default"
        m = re.fullmatch(r"@ByteArray\((.*)\)", raw)
        profile = m.group(1) if m else raw
    rng = random.Random(args.seed)
    start = read(args.instance, profile)
    got = strata_list(strata, args.instance, profile)
    if got != start:
        return fail("before any operation", start, got)
    model = Model(start)
    for step in range(args.steps):
        # names starting with '-' would read as options on the command line
        names = [e[0] for e in model.entries if not e[0].startswith("-")]
        pick = lambda k: rng.sample(names, min(k, len(names)))
        op = rng.choice(["enable", "disable", "move1", "move", "move", "shift"])
        if op in ("enable", "disable"):
            chosen = pick(rng.randint(1, 3))
            cmd = [op] + chosen
            model.set_enabled(chosen, op == "enable")
        elif op == "move1":
            chosen = pick(1)
            to = rng.choice([rng.randint(0, len(names)), 0, len(names) + 5])
            cmd = ["move"] + chosen + ["--to", str(to)]
            model.change_priority(chosen, to)
        elif op == "move":
            chosen = pick(rng.randint(2, 5))
            to = rng.choice([rng.randint(0, len(names)), rng.randint(0, len(names)), 10**9])
            cmd = ["move"] + chosen + ["--to", str(to)]
            model.change_priority(chosen, to)
        else:
            chosen = pick(rng.randint(1, 3))
            by = rng.choice([-3, -1, 1, 2, 7])
            cmd = ["move"] + chosen + ["--by", str(by)]
            model.shift(chosen, by)
        run = subprocess.run([strata, *cmd, "-i", args.instance, "-p", profile], capture_output=True, text=True)
        if run.returncode != 0:
            print(f"step {step}: strata {' '.join(cmd)} failed:\n{run.stderr}")
            return 1
        on_disk = read(args.instance, profile)
        got = strata_list(strata, args.instance, profile)
        if got != on_disk:
            return fail(f"step {step} ({' '.join(cmd)}): Strata vs MO2 reading the files", on_disk, got)
        if model.as_tuples() != on_disk:
            return fail(f"step {step} ({' '.join(cmd)}): MO2's operations vs MO2 reading the files", model.as_tuples(), on_disk)
    print(f"ok: {args.steps} operations on {len(start)} mods (profile {profile}, seed {args.seed})")
    return 0


def fail(what, expected, got):
    print(f"MISMATCH {what}")
    for i in range(max(len(expected), len(got))):
        a = expected[i] if i < len(expected) else None
        b = got[i] if i < len(got) else None
        if a != b:
            print(f"  first difference at priority {i}: expected {a}, got {b}")
            break
    return 1


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("instance")
    c.add_argument("--profile")
    c.add_argument("--steps", type=int, default=200)
    c.add_argument("--seed", type=int, default=1)
    c.add_argument("--strata", default=os.path.join(os.path.dirname(__file__), "../../target/release/strata-headless"))
    r = sub.add_parser("read")
    r.add_argument("instance")
    r.add_argument("profile")
    args = ap.parse_args()
    if args.cmd == "read":
        for p, e in enumerate(read(args.instance, args.profile)):
            print(p, *e, sep="\t")
        return 0
    return check(args)


if __name__ == "__main__":
    sys.exit(main())
