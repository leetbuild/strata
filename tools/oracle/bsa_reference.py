#!/usr/bin/env python3
"""Reference for BSA reading: Fluorine's two readers.

usage: bsa_reference.py <liblibbsarch.so> <archive> <strata listing.json> <strata extract folder>
                        <ba2 listing file, or - when ba2 could not read the archive>

- The `ba2` crate (Fluorine's libs/bsa_ffi; tools/oracle/ba2-list) lists the archive: its list
  must equal Strata's.
- bsatk, the reader MO2's archive parsing uses (DirectoryEntry::addFromBSA), through Fluorine's
  Linux libbsarch (libs/libbsarch/src/libbsarch.cpp). It counts files, finds a path and extracts
  one file, but cannot list (bsa_iterate_files is a stub there). Its count must equal Strata's,
  and every file it finds must have the bytes Strata extracted. Its Linux findFile misses some
  files (the top-level files of a folder that also has subfolders, e.g. interface/hudmenu.swf);
  those are counted and must then be in ba2's list.
Every path Strata lists must be confirmed by ba2's list or by bsatk. Bytes are compared for every
file of archives up to 2,000 files; for bigger ones, for 2,000 files spread evenly (bsatk's
findFile scans the whole archive on every call). Prints one verdict line;
exits 1 on a mismatch.

Known bug of the reference, allowed for exactly: Fluorine's bsatk `Archive::extractDirect` writes
an uncompressed TES4 file, then falls through into the old copy loop and writes `m_FileSize` more
bytes (the file again, or with an embedded name the bytes after it). So for a file Strata lists as
uncompressed, bsatk's bytes may be Strata's followed by exactly `stored_size` more bytes; these are
counted and reported, never silently passed.
"""
import json
import ctypes
import os
import sys
import warnings

warnings.filterwarnings("ignore", category=DeprecationWarning)


class Msg(ctypes.Structure):
    _pack_ = 1
    _fields_ = [("code", ctypes.c_int8), ("text", ctypes.c_wchar * 1024)]


class Buf(ctypes.Structure):
    _pack_ = 1
    _fields_ = [("size", ctypes.c_uint32), ("data", ctypes.c_void_p)]


class MsgBuf(ctypes.Structure):
    _pack_ = 1
    _fields_ = [("buffer", Buf), ("message", Msg)]


def main():
    lib_path, archive, listing, extracted, ba2_listing = sys.argv[1:6]
    lib = ctypes.CDLL(lib_path)
    lib.bsa_create.restype = ctypes.c_void_p
    lib.bsa_load_from_file.restype = Msg
    lib.bsa_load_from_file.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
    lib.bsa_file_count_get.restype = ctypes.c_uint32
    lib.bsa_file_count_get.argtypes = [ctypes.c_void_p]
    lib.bsa_file_exists.restype = ctypes.c_bool
    lib.bsa_file_exists.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
    lib.bsa_extract_file_data_by_filename.restype = MsgBuf
    lib.bsa_extract_file_data_by_filename.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
    lib.bsa_file_data_free.restype = Msg
    lib.bsa_file_data_free.argtypes = [ctypes.c_void_p, Buf]

    handle = lib.bsa_create()
    if lib.bsa_load_from_file(handle, archive).code != 0:
        print(f"FAILED {archive}: bsatk cannot read it")
        sys.exit(1)
    with open(listing, encoding="utf-8") as f:
        files = json.load(f)["files"]
    paths = [f["path"] for f in files]
    info = {f["path"]: f for f in files}
    doubled = 0
    unfound = 0
    ba2_paths = None
    if ba2_listing != "-":
        with open(ba2_listing, encoding="utf-8") as f:
            ba2_paths = sorted(line.rstrip("\n") for line in f if line.strip())
    problems = []
    if ba2_paths is not None and ba2_paths != sorted(paths):
        missing = sorted(set(ba2_paths) - set(paths))[:3]
        extra = sorted(set(paths) - set(ba2_paths))[:3]
        problems.append(f"ba2 lists {len(ba2_paths)} files, Strata {len(paths)}; only ba2: {missing}, only Strata: {extra}")
    count = lib.bsa_file_count_get(handle)
    if count != len(paths):
        problems.append(f"bsatk counts {count} files, Strata lists {len(paths)}")
    checked = 0
    step = max(1, len(paths) // 2000)
    for path in paths[::step]:
        windows = path.replace("/", "\\")
        if not lib.bsa_file_exists(handle, windows):
            if ba2_paths is None:
                problems.append(f"neither bsatk nor ba2 confirms {path}")
            unfound += 1
            continue
        result = lib.bsa_extract_file_data_by_filename(handle, windows)
        if result.message.code != 0:
            problems.append(f"bsatk cannot extract {path}")
            continue
        theirs = ctypes.string_at(result.buffer.data, result.buffer.size) if result.buffer.size else b""
        # (not freed: a packed struct by value does not cross ctypes reliably; the process is short)
        try:
            with open(os.path.join(extracted, path), "rb") as f:
                ours = f.read()
        except OSError as e:
            problems.append(f"Strata did not extract {path}: {e}")
            continue
        if ours != theirs:
            meta = info[path]
            if not meta["compressed"] and theirs.startswith(ours) and len(theirs) == len(ours) + meta["size"]:
                doubled += 1
            else:
                problems.append(f"{path}: {len(ours)} bytes from Strata, {len(theirs)} from bsatk, content differs")
        checked += 1
    if problems:
        print(f"FAILED {archive}: {len(problems)} problems; first: " + "; ".join(problems[:3]))
        sys.exit(1)
    notes = []
    if ba2_paths is None:
        notes.append("ba2 could not read it")
    if unfound:
        notes.append(f"{unfound} not found by bsatk, in ba2's list")
    if doubled:
        notes.append(f"{doubled} uncompressed files doubled by bsatk's extractDirect bug")
    note = f" ({'; '.join(notes)})" if notes else ""
    print(f"ok {len(paths)} files listed, {checked} compared byte for byte{note}, {archive}")


if __name__ == "__main__":
    main()
