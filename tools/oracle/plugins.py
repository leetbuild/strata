#!/usr/bin/env python3
"""Reference plugin list for oracle check 4 (docs/DESIGN.md, "Oracle checks").

A second implementation of MO2 2.5.2's plugin list, written straight from the C++
(modorganizer/src/pluginlist.cpp: refresh, setLoadOrder, setState, fixPrimaryPlugins,
fixPluginRelationships, setPluginPriority, testMasters, readLockedOrderFrom,
refreshLoadOrder, generatePluginIndexes; Fluorine libs/game_bethesda: CreationGamePlugins and
GamebryoGamePlugins readers; libs/esptk for the TES4 header). It shares no code with Strata.

The plugins of the merged Data folder come from the reference merged tree (merge.py, check
2a). Prints one line per plugin, by priority:
  priority  name  +/-  load order  index  master  light  masters  missing masters
tools/oracle/check-plugins.sh compares it with `strata list plugins --json`.

Where MO2's result is unspecified (it walks a hash map for plugins the files don't name),
this reference does what Strata promises: those plugins go last, by name.

usage: plugins.py <instance folder> [profile]
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import merge  # noqa: E402  (the check 2a reference: merged tree)

SSE_PRIMARY = ["skyrim.esm", "update.esm", "dawnguard.esm", "hearthfires.esm", "dragonborn.esm"]
# Skyrim LE (game_skyrim): two primary plugins, no light plugins, Gamebryo plugins.txt.
LE_PRIMARY = ["skyrim.esm", "update.esm"]
OBLIVION_PRIMARY = ["oblivion.esm", "update.esm"]


def low(s):
    return s.lower()


def read_header(path):
    """(flags, masters) of a TES4 header, or None when esptk would throw."""
    try:
        with open(path, "rb") as f:
            head = f.read(24)
            if len(head) < 24 or head[:4] != b"TES4":
                return None
            size, flags = struct.unpack_from("<II", head, 4)
            if head[20:24] == b"HEDR":
                body = head[20:24] + f.read(size - 4)
            else:
                body = f.read(size)
    except OSError:
        return None
    if len(body) < size:
        return None
    masters, i, override = [], 0, None
    while i < len(body):
        if i + 6 > len(body):
            return None
        kind = body[i:i + 4]
        n = struct.unpack_from("<H", body, i + 4)[0]
        i += 6
        if override is not None:
            n, override = override, None
        data = body[i:i + n]
        if len(data) < n:
            return None
        i += n
        if kind == b"XXXX":
            override = struct.unpack_from("<I", data)[0]
        elif kind == b"MAST" and n:
            masters.append(data.split(b"\0")[0].decode("utf-8", "replace"))
    return flags, masters


class P:
    def __init__(self, name, path, primary, force_core, light_ok=True):
        self.name = name
        h = read_header(path)
        ext = name[-4:].lower()
        self.master_ext = h is not None and ext == ".esm"
        self.light_ext = h is not None and ext == ".esl"
        self.master_flag = h is not None and bool(h[0] & 1)
        self.light_flag = light_ok and h is not None and bool(h[0] & 0x200)
        # an .esl where light plugins are unknown can never be enabled (refresh(), endsWith(".esl"))
        self.force_disabled = not light_ok and name.endswith(".esl")
        ms = {}
        for m in (h[1] if h else []):
            ms.setdefault(low(m), m)
        self.masters = [ms[k] for k in sorted(ms)]
        self.forced = force_core and low(name) in {low(x) for x in primary}
        self.enabled = self.forced
        self.prio = -1
        self.lo = -1
        self.index = ""
        self.unset = []

    def masterish(self):
        return self.light_ext or self.master_ext or self.master_flag


class List:
    def __init__(self, plugins, primary, force_core, light_ok=True):
        self.light_ok = light_ok
        self.esps = plugins  # rows
        self.primary = primary
        self.force_core = force_core
        self.locked = {}  # low name -> (name, position)

    def reindex(self):
        self.by_name = {low(p.name): i for i, p in enumerate(self.esps)}
        self.by_prio = [0] * len(self.esps)
        for i, p in enumerate(self.esps):
            if 0 <= p.prio < len(self.esps):
                self.by_prio[p.prio] = i
        lights = skipped = 0
        for l, i in enumerate(self.by_prio):
            p = self.esps[i]
            if not p.enabled:
                p.index = ""
                skipped += 1
            elif self.light_ok and (p.light_ext or p.light_flag):
                p.index = "%02X:%03X" % (254 + lights // 4096, lights % 4096)
                lights += 1
            else:
                p.index = "%02X" % (l - lights - skipped)

    def find(self, name):
        return self.by_name.get(low(name))

    def set_state(self, name, active):
        i = self.find(name)
        if i is not None:
            p = self.esps[i]
            p.enabled = (active and not p.force_disabled) or p.forced

    def set_load_order(self, names):
        for p in self.esps:
            p.prio = -1
        n = 0
        for name in names:
            i = self.find(name)
            if i is not None and self.esps[i].prio == -1:
                self.esps[i].prio = n
                n += 1
        for p in self.esps:
            if p.prio == -1:
                p.prio = n
                n += 1
        self.reindex()

    def read_plugins_txt_le(self, data):
        """SkyrimGamePlugins::readPluginList: primary on, every listed name on, the rest off."""
        prim = {low(x) for x in self.primary}
        for name in self.primary:
            self.set_state(name, True)
        listed = set()
        for raw in (data or b"").split(b"\n"):
            if raw[:1] not in (b"", b"#"):
                name = raw.strip().decode("utf-8", "replace")
                if name:
                    self.set_state(name, True)
                    listed.add(low(name))
        for p in self.esps:
            if low(p.name) not in prim and low(p.name) not in listed:
                self.set_state(p.name, False)

    def read_plugins_txt(self, data):
        order = list(self.primary)
        prim = {low(x) for x in self.primary}
        for name in self.primary:
            self.set_state(name, True)
        if not data:
            return order
        found = set()
        for raw in data.split(b"\n"):
            name = raw.strip().decode("utf-8", "replace") if raw[:1] not in (b"", b"#") else ""
            if low(name) not in prim:
                active = name.startswith("*")
                if active:
                    name = name[1:]
                if name:
                    self.set_state(name, active)
                    found.add(low(name))
                    if low(name) not in {low(x) for x in order}:
                        order.append(name)
            else:
                found.add(low(name[1:]))
        for p in self.esps:
            if low(p.name) not in found:
                self.set_state(p.name, False)
        return order

    def set_priority(self, row, newp):
        """setPluginPriority; returns True when the plugin moved up."""
        n = len(self.by_prio)
        t = max(0, min(newp, n - 1))
        me = self.esps[row]
        at = lambda k: self.esps[self.by_prio[k]]  # noqa: E731
        if not me.masterish():
            while t < n - 1 and at(t).masterish():
                t += 1
        else:
            while t > 0 and not at(t).masterish():
                t -= 1
            while t < n - 1 and at(t).forced:
                t += 1
        old = me.prio
        if t < old:
            for m in me.masters:
                j = self.find(m)
                if j is not None and self.esps[j].prio >= t:
                    t = self.esps[j].prio + 1
        elif t > old:
            k = old + 1
            while k <= t:
                o = at(k)
                if any(low(m) == low(me.name) for m in o.masters):
                    t = o.prio - 1
                k += 1
        up = False
        if t != old and 0 <= t < n:
            if t > old:
                for k in range(old + 1, t + 1):
                    at(k).prio -= 1
            else:
                for k in range(t, old):
                    at(k).prio += 1
                up = True
            me.prio = t
        self.reindex()
        return up

    def fix_primary(self):
        if not self.force_core:
            return
        prio = 0
        for name in self.primary:
            i = self.find(name)
            if i is not None:
                if self.esps[i].prio != prio:
                    self.set_priority(i, prio)
                prio += 1

    def fix_relationships(self):
        count = sum(1 for p in self.esps if p.masterish())
        for i, p in enumerate(self.esps):
            if p.masterish() and p.prio > count or not p.masterish() and p.prio < count:
                self.set_priority(i, count + 1)
        for i, p in enumerate(self.esps):
            newp = p.prio
            for m in p.masters:
                j = self.find(m)
                if j is not None:
                    newp = max(newp, self.esps[j].prio)
            if newp != p.prio:
                self.set_priority(i, newp)

    def test_masters(self):
        on = {low(p.name) for p in self.esps if p.enabled}
        for p in self.esps:
            p.unset = [m for m in p.masters if low(m) not in on] if p.enabled else []

    def sync_lo(self):
        n = 0
        for i in self.by_prio:
            p = self.esps[i]
            if p.enabled:
                p.lo = n
                n += 1
            else:
                p.lo = -1

    def read_locked(self, data):
        self.locked = {}
        n = len(self.by_prio)
        for line in data.split(b"\n"):
            if line[:1] == b"#":
                continue
            fields = line.split(b"|")
            if len(fields) != 2:
                continue
            name = fields[0].decode("utf-8", "replace")
            try:
                pos = int(fields[1].strip())
            except ValueError:
                pos = 0
            if pos < 0:
                continue
            i = self.find(name)
            if i is None:
                self.locked[low(name)] = (name, pos)
                continue
            if self.esps[i].forced:
                continue
            if pos >= n:
                self.locked[low(name)] = (name, pos)
                continue

            def free(k):
                return not self.esps[self.by_prio[k]].forced and all(v != k for _, v in self.locked.values())
            if free(pos):
                self.locked[low(name)] = (name, pos)
                continue
            pos += 1
            while pos < n and not free(pos):
                pos += 1
            self.locked[low(name)] = (name, pos)

    def refresh_load_order(self):
        self.sync_lo()
        by_pos = {}
        for k in sorted(self.locked):
            name, pos = self.locked[k]
            by_pos[pos] = name
        n = len(self.esps)
        t = 0
        for pos in sorted(by_pos):
            i = self.find(by_pos[pos])
            if i is None:
                continue
            while t < n - 1 and self.esps[self.by_prio[t]].lo < pos:
                t += 1
            if t >= n:
                continue
            if self.esps[i].prio != t:
                self.set_priority(i, t)
                self.esps[i].lo = pos
                self.sync_lo()
        self.reindex()


def read(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except FileNotFoundError:
        return None


def lines(data):
    for raw in data.replace(b"\r", b"\n").split(b"\n"):
        s = raw.strip()
        if s and not s.startswith(b"#"):
            yield s.decode("utf-8", "replace")


def build(instance, profile=None):
    with open(os.path.join(instance, "ModOrganizer.ini"), encoding="utf-8", errors="replace") as f:
        ini = f.read()
    profile = profile or merge.ini_value(ini, "General", "selected_profile")
    game = merge.host_path(merge.ini_value(ini, "General", "gamePath"))
    force = (merge.ini_value(ini, "Settings", "force_enable_core_files") or "true").lower() == "true"
    game_name = (merge.ini_value(ini, "General", "gameName") or "").lower()
    # Skyrim LE and Oblivion: GamebryoGamePlugins lists, no light plugins, no Creation Club.
    le = game_name in ("skyrim", "oblivion")
    primary = list(OBLIVION_PRIMARY if game_name == "oblivion" else LE_PRIMARY if le else SSE_PRIMARY)
    ccc = None if le else read(os.path.join(game, "Skyrim.ccc"))
    for name in lines(ccc or b""):
        if low(name) not in {low(x) for x in primary}:
            primary.append(name)

    # the merged tree, as check 2a's reference builds it
    tree = merge.Tree()
    rules = [("", ["Data"], merge.MOD_EXCLUDE), ("Root", [], merge.root_exclude(merge.ini_value(ini, "General", "gameName")))]
    tree.add_layer(game, "<game>", [("", [], set())])
    layers = {"<game>": game, "<overwrite>": os.path.join(instance, "overwrite")}
    for name in merge.enabled_mods(instance, profile):
        layers[name] = os.path.join(instance, "mods", name)
        tree.add_layer(layers[name], name, rules)
    wo = read(os.path.join(instance, "strata", "whiteouts.txt"))
    for line in (wo or b"").decode("utf-8").splitlines():
        if line.strip() and not line.startswith("#"):
            tree.hide(line.strip())
    tree.add_layer(layers["<overwrite>"], "<overwrite>", rules)

    found = []
    for key, origin in tree.files.items():
        parts = key.split("/")
        if len(parts) == 2 and parts[0] == "data" and parts[1][-4:] in (".esp", ".esm", ".esl") and len(parts[1]) > 4:
            shown = tree.names[key]
            # a mod's Data files sit at its top level; the game's in its Data folder
            data_dir = merge.find_dir(game, "Data") if origin == "<game>" else layers[origin]
            real = None
            for e in os.scandir(data_dir):
                if e.name.lower() == parts[1] and e.is_file():
                    real = e.path
            found.append((shown, real))
    found.sort(key=lambda x: low(x[0]))

    esps = [P(name, path, primary, force, not le) for name, path in found]
    lst = List(esps, primary, force, not le)
    for i, p in enumerate(esps):
        p.prio = i
    lst.reindex()
    pdir = os.path.join(instance, "profiles", profile)
    plugins_txt = read(os.path.join(pdir, "plugins.txt"))
    lo = read(os.path.join(pdir, "loadorder.txt"))
    if lo is not None:
        order = list(primary)
        for name in lines(lo):
            if low(name) not in {low(x) for x in order}:
                order.append(name)
    elif le:
        order = [lst.esps[i].name for i in lst.by_prio]
    else:
        order = lst.read_plugins_txt(plugins_txt)
    lst.set_load_order(order)
    if le:
        lst.read_plugins_txt_le(plugins_txt)
    else:
        lst.read_plugins_txt(plugins_txt)
    lst.fix_primary()
    lst.fix_relationships()
    lst.test_masters()
    lst.reindex()
    locked = read(os.path.join(pdir, "lockedorder.txt"))
    if locked is not None:
        lst.read_locked(locked)
    lst.refresh_load_order()
    lst.test_masters()
    return lst


def main():
    lst = build(os.path.abspath(sys.argv[1]), sys.argv[2] if len(sys.argv) > 2 else None)
    for i in lst.by_prio:
        p = lst.esps[i]
        print("\t".join([str(p.prio), p.name, "+" if p.enabled else "-", str(p.lo), p.index,
                         "M" if p.masterish() else "-", "L" if p.light_ext or p.light_flag else "-",
                         ",".join(p.masters), ",".join(p.unset)]))


if __name__ == "__main__":
    main()
