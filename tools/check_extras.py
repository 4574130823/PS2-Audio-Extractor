"""Checks the scanner's extras (video demuxing, compressed data, headerless audio) against
vgmstream's decode of the same sounds as standalone files.

    python tools/check_extras.py
"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check  # noqa: E402

TOOLS = check.TOOLS
data = os.path.join(TOOLS, "testdata", "extras")
ref = os.path.join(TOOLS, "testdata", "extras_ref")
tmp = tempfile.mkdtemp()
vgm = check.vgm_tracks(ref, tmp)
out = os.path.join(TOOLS, "out", "extras")
import shutil, subprocess, csv  # noqa: E401,E402
shutil.rmtree(out, ignore_errors=True)
exe = os.path.join(check.ROOT, "target", "debug", "ps2audioextractor.exe")
r = subprocess.run([exe, data, out], capture_output=True, text=True)
print(r.stdout[-3000:])
game = os.path.join(out, os.listdir(out)[0])
rows = list(csv.DictReader(open(os.path.join(game, "tracks.csv"), encoding="utf-8")))
for row in rows:
    row["wav"] = os.path.join(game, *row["file"].split("/"))
sys.exit(1 if check.compare("extras", vgm, rows) else 0)
