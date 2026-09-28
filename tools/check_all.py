"""Builds every test fixture and checks everything against vgmstream.

    python tools/check_all.py [--no-build-fixtures]

Needs tools/vgmstream/vgmstream-cli.exe (a vgmstream release) and a debug build
(`cargo build`). Exits non-zero if anything differs.
"""
import os
import subprocess
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(TOOLS, "fixtures")

# (fixture script, test folder, extensions that can't be found inside other files)
SUITES = [
    ("core.py", "core", "hd,bd"),
    ("simple.py", "simple", "vgv,seb,gms,mic"),
    ("assorted.py", "assorted", "int,wp2,pcm,voi,xa2,msa,sre,hd2,bd"),
    ("streams.py", "streams", "vsv,mjh,msh,mib,mic,vgs,sts,x,sts_cp3,imc,joe,adm,hxd,xwb,rkv,vas,skx"),
    ("codecs.py", "codecs", "sfx,sf0,adp,rsp,str,mul,smp"),
    ("ea_ubi.py", "ea_ubi", "abk,amb,hdr,mpf,map,msb,hx2,hxc,sb1,sm1,bnm"),
    ("major.py", "major", "swag,vig,vas,str,vag,800,hd,bd,acx,aac,laac"),
    ("dual.py", "dual", None),
    ("final.py", "final", None),
]


def run(args):
    return subprocess.run([sys.executable] + args, capture_output=True, text=True)


def main():
    build = "--no-build-fixtures" not in sys.argv
    failed = []
    for script, folder, no_embed in SUITES:
        if build:
            r = run([os.path.join(FIX, script)])
            if r.returncode != 0:
                print(f"{script}: fixture build failed\n{r.stderr[-1500:]}")
                failed.append(folder)
                continue
        args = [os.path.join(TOOLS, "check.py"), os.path.join(TOOLS, "testdata", folder)]
        if no_embed is not None:
            args += ["--embed", "--no-embed-ext", no_embed]
        r = run(args)
        lines = r.stdout.strip().splitlines()
        for line in lines:
            if not line.lstrip().startswith("OK"):
                print(line)
        if r.returncode != 0:
            failed.append(folder)
    if build:
        run([os.path.join(FIX, "extras.py")])
    r = run([os.path.join(TOOLS, "check_extras.py")])
    print(r.stdout.strip().splitlines()[-1] if r.stdout.strip() else r.stderr[-800:])
    if r.returncode != 0:
        failed.append("extras")
    print("\nALL CHECKS PASS" if not failed else f"\nFAILED: {', '.join(failed)}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
