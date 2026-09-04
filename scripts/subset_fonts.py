#!/usr/bin/env python3
"""Subset Noto Sans SC to exactly the glyphs this program uses.

Why: egui renders with bundled fonts + system CJK fallback. Linux-minimal
systems often lack a system CJK font, so the UI would show tofu. Embedding
only the used subset (~hundreds of KB) keeps every platform covered without
bloating the single binary; rare user-content chars still fall back to the
system font at runtime (see install_cjk_font in app-egui).

Full font (OFL, ~16MB, NOT committed) e.g.:
  curl -L -o /tmp/NotoSansCJKsc-Regular.otf \\
    https://github.com/notofonts/noto-cjk/raw/main/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf

Usage (from repo root):
    python3 scripts/subset_fonts.py [--full-font /tmp/NotoSansCJKsc-Regular.otf]

Re-run whenever shared/locales/*.yml or built-in UI/template strings change.
"""
import argparse
import glob
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "fonts", "NotoSansSC-subset.otf")


def collect_chars():
    pats = ["shared/locales/*.yml", "app-egui/src/main.rs",
            "templates/*.json", "registry/*.json", "demo/shell.json"]
    files = [f for p in pats for f in glob.glob(os.path.join(ROOT, p))]
    chars = set()
    for f in files:
        try:
            text = open(f, encoding="utf-8").read()
        except OSError as e:
            print(f"skip {f}: {e}")
            continue
        for ch in text:
            if ('\u4e00' <= ch <= '\u9fff' or '\u3400' <= ch <= '\u4dbf'
                    or '\uf900' <= ch <= '\ufaff'
                    or '\u3000' <= ch <= '\u303f'
                    or '\uff00' <= ch <= '\uffef'):
                chars.add(ch)
    return files, chars


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--full-font", default="/tmp/NotoSansCJKsc-Regular.otf")
    args = ap.parse_args()
    if not os.path.isfile(args.full_font):
        print(f"full font not found: {args.full_font}\nsee header for download URL")
        return 1

    from fontTools import subset

    files, chars = collect_chars()
    unicodes = sorted(ord(c) for c in chars)
    print(f"scanned {len(files)} files, {len(chars)} unique chars")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    opts = subset.Options()
    opts.name_IDs = ["*"]  # drop name table bloat? keep minimal: family names only
    opts.name_IDs = [1, 2, 4, 6, 16, 17]
    font = subset.load_font(args.full_font, opts)
    ss = subset.Subsetter(opts)
    ss.populate(unicodes=unicodes)
    ss.subset(font)
    font.save(OUT)

    # verify 100% coverage of used chars
    cmap = font.getBestCmap()
    missing = [c for c in chars if ord(c) not in cmap]
    full = os.path.getsize(args.full_font)
    small = os.path.getsize(OUT)
    print(f"full: {full / 1e6:.1f}MB -> subset: {small / 1e3:.0f}KB "
          f"({100.0 * small / full:.1f}%)")
    if missing:
        print(f"MISSING {len(missing)}: {''.join(sorted(missing))}")
        return 1
    print(f"coverage 100%, wrote {os.path.relpath(OUT, ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
