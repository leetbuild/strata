#!/usr/bin/env python3
"""Prints the CLI's commands as the Markdown list in docs/ARCHITECTURE.md ("CLI"), from
`strata --help` and each subcommand's help.

usage: tools/cli-help.py [binary]   (default: target/debug/strata-headless)
"""
import re
import subprocess
import sys

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/debug/strata-headless"
GLOBAL = ("--instance", "--profile", "--json", "--prefix", "--help")


def help_text(args):
    return subprocess.run([BIN, *args, "--help"], capture_output=True, text=True).stdout


def commands(text):
    m = re.search(r"Commands:\n(.*?)\n\n", text, re.S)
    out = []
    for line in (m.group(1).split("\n") if m else []):
        mm = re.match(r"\s{2}(\S+)\s+(.*)", line)
        if mm and mm.group(1) != "help":
            out.append((mm.group(1), mm.group(2)))
    return out


def usage(text):
    u = re.search(r"Usage: \S+ (.*)", text).group(1)
    return re.sub(r"\[OPTIONS\]\s*", "", u).strip()


for name, desc in commands(help_text([])):
    text = help_text([name])
    subs = commands(text)
    if subs:
        print(f"- `strata {name}`: {desc}")
        for sub, sub_desc in subs:
            print(f"  - `strata {usage(help_text([name, sub]))}`: {sub_desc}")
    else:
        opts = [o for o in re.findall(r"^\s+(?:-\w, )?(--[a-z-]+)", text, re.M) if o not in GLOBAL]
        extra = f" [{' '.join(opts)}]" if opts else ""
        print(f"- `strata {usage(text)}`{extra}: {desc}")
