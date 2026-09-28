"""Checks ps2audioextractor against vgmstream on a folder of test files.

    python tools/check.py tools/testdata/<name> [--embed] [--exe path]

Every file vgmstream can open is decoded with it (each subsong, loops ignored), and must
match one of our tracks exactly: same channels, rate, length, loop points and samples.
Tracks of ours that vgmstream doesn't have are reported as extra.

--embed also buries each file vgmstream opened in random data (at odd, 16-aligned
offsets) and checks the scanner still finds and decodes it the same way. Formats that
need a second file (header + data) or are known only by extension can't pass that part;
list their files with --no-embed-ext EXT[,EXT...].
"""
import argparse
import csv
import hashlib
import json
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(TOOLS)
VGM = os.path.join(TOOLS, "vgmstream", "vgmstream-cli.exe")


def read_wav(path):
    d = open(path, "rb").read()
    if d[:4] != b"RIFF" or d[8:12] != b"WAVE":
        raise ValueError(f"not a WAV: {path}")
    pos, ch, rate, data = 12, None, None, b""
    while pos + 8 <= len(d):
        cid, size = d[pos:pos + 4], struct.unpack("<I", d[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            _, ch, rate = struct.unpack("<HHI", d[pos + 8:pos + 16])
        elif cid == b"data":
            data = d[pos + 8:pos + 8 + size]
            break
        pos += 8 + size + (size & 1)
    return ch, rate, data


def vgm_tracks(folder, tmp):
    """(file, subsong, info, wav path) for everything vgmstream decodes in `folder`."""
    out = []
    for dirpath, _, names in os.walk(folder):
        for n in sorted(names):
            path = os.path.join(dirpath, n)
            rel = os.path.relpath(path, folder).replace(os.sep, "/")
            r = subprocess.run([VGM, "-I", "-m", path], capture_output=True, text=True)
            if r.returncode != 0 or not r.stdout.strip().startswith("{"):
                continue
            info = json.loads(r.stdout.strip().splitlines()[0])
            total = (info.get("streamInfo") or {}).get("total") or 0
            for sub in range(1, max(total, 1) + 1):
                args = ["-s", str(sub)] if total else []
                wav = os.path.join(tmp, f"vgm_{len(out)}.wav")
                r = subprocess.run([VGM, "-i", "-o", wav] + args + [path], capture_output=True, text=True)
                if r.returncode != 0:
                    print(f"  (vgmstream couldn't decode {rel} #{sub}: {r.stderr.strip()[:120]})")
                    continue
                if total:
                    i = subprocess.run([VGM, "-I", "-m", "-s", str(sub), path], capture_output=True, text=True)
                    info = json.loads(i.stdout.strip().splitlines()[0])
                out.append((rel, sub if total else 0, info, wav))
    return out


def ours(exe, folder, name):
    """Runs the extractor: list of track dicts (from tracks.csv) with the WAV path."""
    out = os.path.join(TOOLS, "out", name)
    shutil.rmtree(out, ignore_errors=True)
    r = subprocess.run([exe, folder, out, "--no-headerless"], capture_output=True, text=True)
    if r.returncode not in (0, 2):
        print(r.stdout[-2000:], r.stderr[-2000:])
        raise SystemExit(f"extractor failed ({r.returncode})")
    games = [d for d in os.listdir(out) if os.path.isdir(os.path.join(out, d))]
    game = os.path.join(out, games[0])
    rows = list(csv.DictReader(open(os.path.join(game, "tracks.csv"), encoding="utf-8")))
    for row in rows:
        row["wav"] = os.path.join(game, *row["file"].split("/"))
    return rows


def digest(ch, rate, data):
    return (ch, rate, len(data), hashlib.sha1(data).hexdigest())


def compare(label, vgm, mine):
    """Matches vgmstream's tracks with ours by content. Returns number of problems."""
    problems = 0
    by_digest = {}
    for m in mine:
        if m["note"]:
            continue
        try:
            by_digest.setdefault(digest(*read_wav(m["wav"])), []).append(m)
        except FileNotFoundError:
            print(f"  MISSING WAV {m['file']}")
            problems += 1
    used = set()
    for rel, sub, info, wav in vgm:
        ch, rate, data = read_wav(wav)
        cands = [m for m in by_digest.get(digest(ch, rate, data), []) if id(m) not in used]
        tag = f"{rel}" + (f" #{sub}" if sub else "")
        if not cands and by_digest.get(digest(ch, rate, data)):
            # vgmstream opened the other file of a dual-file stereo pair: the same sound.
            print(f"  OK     {tag} -> {by_digest[digest(ch, rate, data)][0]['file']} (other file of a stereo pair)")
            continue
        if not cands:
            # Explain: the track of ours from the same file, in the same position.
            same = [m for m in mine if m["source"].replace("\\", "/").endswith(rel.split("/")[-1])]
            near = same[(sub or 1) - 1] if len(same) >= (sub or 1) else None
            if near and not near["note"] and os.path.exists(near["wav"]):
                c2, r2, d2 = read_wav(near["wav"])
                a = struct.unpack(f"<{len(data)//2}h", data)
                b = struct.unpack(f"<{len(d2)//2}h", d2)
                first = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), None)
                print(f"  DIFF   {tag}: vgmstream {ch}ch {rate}Hz {len(a)//max(ch,1)} samples | ours {c2}ch {r2}Hz {len(b)//max(c2,1)} samples"
                      f" | first differing sample {first}")
            else:
                print(f"  MISSED {tag} ({ch}ch {rate}Hz {len(data)//2//max(ch,1)} samples, {info.get('metadataSource')})"
                      + (f" [ours: {near['note']}]" if near and near["note"] else ""))
            problems += 1
            continue
        m = cands[0]
        used.add(id(m))
        li = info.get("loopingInfo")
        want = (li["start"], li["end"]) if li else (None, None)
        got = (int(m["loop_start"]) if m["loop_start"] else None, int(m["loop_end"]) if m["loop_end"] else None)
        if want != got:
            print(f"  LOOP   {tag}: vgmstream {want} ours {got}")
            problems += 1
        else:
            print(f"  OK     {tag} -> {m['file']}" + (f" loop {got[0]}-{got[1]}" if got[0] is not None else ""))
    for m in mine:
        if id(m) not in used and not m["note"]:
            print(f"  EXTRA  {m['file']} (from {m['source']} @ {m['offset']}, {m['format']})")
            problems += 1
    print(f"{label}: {len(vgm)} vgmstream tracks, {len(mine)} ours, {problems} problem(s)")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("folder")
    ap.add_argument("--embed", action="store_true")
    ap.add_argument("--no-embed-ext", default="")
    ap.add_argument("--exe", default=os.path.join(ROOT, "target", "debug", "ps2audioextractor.exe"))
    a = ap.parse_args()
    folder = os.path.abspath(a.folder)
    name = os.path.basename(folder.rstrip("/\\"))
    tmp = tempfile.mkdtemp()
    vgm = vgm_tracks(folder, tmp)
    problems = compare(name, vgm, ours(a.exe, folder, name))

    if a.embed:
        skip = {e.strip().lower().lstrip(".") for e in a.no_embed_ext.split(",") if e.strip()}
        rnd = random.Random(99)
        files = sorted({rel for rel, *_ in vgm if rel.rsplit(".", 1)[-1].lower() not in skip})
        blob = bytearray(os.urandom(0x1000))
        for rel in files:
            blob += os.urandom(rnd.randint(1, 3000))
            blob += b"\0" * ((-len(blob)) % 16)
            blob += open(os.path.join(folder, *rel.split("/")), "rb").read()
        blob += os.urandom(0x1000)
        edir = os.path.join(TOOLS, "testdata", "_embed_" + name)
        shutil.rmtree(edir, ignore_errors=True)
        os.makedirs(edir)
        open(os.path.join(edir, "ARCHIVE.DAT"), "wb").write(blob)
        wanted = [t for t in vgm if t[0] in files]
        problems += compare(name + " (embedded)", wanted, ours(a.exe, edir, "_embed_" + name))
    shutil.rmtree(tmp, ignore_errors=True)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
