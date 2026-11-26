#!/usr/bin/env python3
"""Audit helper: blanks out #[cfg(test)] items and skips tests/, then
lists by mode: unwrap (unwrap/expect outside tests), todo, long (fns of 150+ lines), ignored.

usage: tools/audit.py unwrap|todo|long|ignored   (from the repository root)
"""
import re,os,sys,subprocess
files=subprocess.check_output(['git','ls-files','crates/*.rs']).decode().split()
def strip_tests(src):
    # blank out #[cfg(test)] items (mod blocks or single fns)
    out=list(src); i=0
    for m in re.finditer(r'#\[cfg\(test\)\]',src):
        j=src.find('{',m.end())
        semi=src.find(';',m.end())
        if semi!=-1 and semi<j: 
            for k in range(m.start(),semi+1): 
                if out[k]!='\n': out[k]=' '
            continue
        depth=0;k=j
        while k<len(src):
            if src[k]=='{':depth+=1
            elif src[k]=='}':
                depth-=1
                if depth==0:break
            k+=1
        for x in range(m.start(),k+1):
            if out[x]!='\n': out[x]=' '
    return ''.join(out)
mode=sys.argv[1]
for f in files:
    if '/tests/' in f or f.endswith('/tests.rs') or '/benches/' in f: continue
    src=open(f).read(); s=strip_tests(src)
    lines=s.split('\n')
    if mode=='unwrap':
        for n,l in enumerate(lines,1):
            if re.search(r'\.unwrap\(\)|\.expect\(',l) and not l.strip().startswith('//'):
                print(f"{f}:{n}: {l.strip()[:150]}")
    elif mode=='todo':
        for n,l in enumerate(src.split('\n'),1):
            if re.search(r'TODO|FIXME|XXX|HACK|todo!\(|unimplemented!\(',l): print(f"{f}:{n}: {l.strip()[:150]}")
    elif mode=='long':
        # fn length by brace matching
        for m in re.finditer(r'^\s*(pub(\([^)]*\))?\s+)?(async\s+)?fn\s+(\w+)',src,re.M):
            j=src.find('{',m.end()); semi=src.find(';',m.end())
            if j==-1 or (semi!=-1 and semi<j and src[m.end():semi].count('(')==src[m.end():semi].count(')')): continue
            depth=0;k=j
            while k<len(src):
                c=src[k]
                if c=='{':depth+=1
                elif c=='}':
                    depth-=1
                    if depth==0:break
                k+=1
            a=src.count('\n',0,m.start())+1; b=src.count('\n',0,k)+1
            if b-a>=150: print(f"{f}:{a}: fn {m.group(4)} {b-a+1} lines")
    elif mode=='ignored':
        for n,l in enumerate(src.split('\n'),1):
            if '#[ignore' in l: print(f"{f}:{n}: {l.strip()}")
