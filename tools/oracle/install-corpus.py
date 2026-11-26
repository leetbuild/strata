#!/usr/bin/env python3
"""Runs a pile of real mod archives through `strata install` and writes a table.

usage: install-corpus.py <instance> <out.tsv> <archive or folder>... [--install] [--jobs N]

Every archive (.7z .zip .rar .fomod, folders searched recursively) goes through
`strata install --plan --json` against <instance> (nothing is written there). With --install
each one is also really installed into <instance> (use a generated instance or a reflink copy:
never a real one) from a symlink in its downloads folder (an install writes the download's
`.meta` next to the path it was given, as MO2's `markInstalled` does), with --rename on a clash so every archive gets its own mod, and the
installed folder is checked against the plan (same file list). The archives are only read.

Columns: archive, MiB, installer, mod name, files planned, result, note. Result is
`plan` / `installed` / `mismatch` / `failed`; the note holds the error or the odd thing seen.
Exits 1 when any archive failed or mismatched.
"""
import argparse
import concurrent.futures as cf
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EXTS = (".7z", ".zip", ".rar", ".fomod")


def archives(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, files in os.walk(p):
                for f in sorted(files):
                    if f.lower().endswith(EXTS):
                        yield os.path.join(root, f)
        elif p.lower().endswith(EXTS):
            yield p


def run(strata, args, timeout):
    try:
        r = subprocess.run([strata, *args], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, f"timed out after {timeout}s"
    if r.returncode != 0:
        err = (r.stderr.strip() or r.stdout.strip()).splitlines()
        return None, err[-1] if err else f"exit {r.returncode}"
    try:
        return json.loads(r.stdout), None
    except json.JSONDecodeError as e:
        return None, f"bad JSON: {e}"


def tree(path):
    out = set()
    for root, dirs, files in os.walk(path):
        rel = os.path.relpath(root, path)
        for d in dirs:
            out.add(os.path.normpath(os.path.join(rel, d)).replace(os.sep, "/") + "/")
        for f in files:
            out.add(os.path.normpath(os.path.join(rel, f)).replace(os.sep, "/"))
    out.discard("meta.ini")
    return out


def plan_one(strata, instance, archive):
    data, err = run(strata, ["install", "-i", instance, "--plan", "--json", archive], 600)
    if err:
        return {"archive": archive, "result": "failed", "note": err}
    plan = data["plan"]
    note = "; ".join(l for l in data.get("log", []) if l)
    return {"archive": archive, "result": "plan", "installer": plan["installer"], "name": plan["name"],
            "files": plan["files"], "exists": plan["exists"], "note": note}


def install_one(strata, instance, row):
    name = row["name"]
    downloads = os.path.join(instance, "downloads")
    os.makedirs(downloads, exist_ok=True)
    archive = os.path.join(downloads, os.path.basename(row["archive"]))
    if not os.path.lexists(archive):
        os.symlink(os.path.abspath(row["archive"]), archive)
    args = ["install", "-i", instance, "--json", archive]
    if row["exists"] or os.path.isdir(os.path.join(instance, "mods", name)):
        base = os.path.splitext(os.path.basename(archive))[0]
        args[3:3] = ["--rename", f"{name} ({base})"]
    data, err = run(strata, args, 3600)
    if err:
        row["result"], row["note"] = "failed", err
        return row
    mod = data["installed"]["name"]
    got = tree(os.path.join(instance, "mods", mod))
    want = set(row["files"]) - {"meta.ini"}
    if got != want:
        extra, missing = sorted(got - want), sorted(want - got)
        row["result"] = "mismatch"
        row["note"] = f"extra {extra[:3]} missing {missing[:3]}"
    else:
        row["result"] = "installed"
    return row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("instance")
    ap.add_argument("out")
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--install", action="store_true")
    ap.add_argument("--jobs", type=int, default=3)
    a = ap.parse_args()
    strata = os.environ.get("STRATA", os.path.join(HERE, "target/release/strata-headless"))
    files = list(archives(a.paths))
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        rows = list(ex.map(lambda f: plan_one(strata, a.instance, f), files))
    if a.install:
        # one at a time: installs write the same mod list
        rows = [install_one(strata, a.instance, r) if r["result"] == "plan" else r for r in rows]
    bad = 0
    with open(a.out, "w", encoding="utf-8", newline="\n") as out:
        out.write("archive\tMiB\tinstaller\tname\tfiles\tresult\tnote\n")
        for r in rows:
            mib = os.path.getsize(r["archive"]) / 1048576
            files = sum(1 for f in r.get("files", []) if not f.endswith("/"))
            bad += r["result"] in ("failed", "mismatch")
            note = r.get("note", "").replace("\t", " ").replace("\n", " ")
            out.write(f"{os.path.basename(r['archive'])}\t{mib:.1f}\t{r.get('installer', '')}\t"
                      f"{r.get('name', '')}\t{files}\t{r['result']}\t{note}\n")
    counts = {}
    for r in rows:
        counts[(r["result"], r.get("installer", ""))] = counts.get((r["result"], r.get("installer", "")), 0) + 1
    for k, v in sorted(counts.items()):
        print(f"{k[0]:9} {k[1]:10} {v}")
    print(f"{len(rows)} archives, {bad} failed or mismatched -> {a.out}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
