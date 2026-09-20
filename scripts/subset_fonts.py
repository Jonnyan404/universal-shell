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
                                    [--templates-dir DIR]... [--no-fetch-registry]

Templates moved to their own repo (universal-shell-templates), so template text
(template names/descriptions/field labels shown in the UI) is collected from:
  1. `--templates-dir` (repeatable) or $US_TEMPLATES_DIR, if given;
  2. `../universal-shell-templates/templates/` when the two repos sit side by side;
  3. otherwise the published manifest (manifests.json) is fetched from the raw URL
     below — needs network; skip it with --no-fetch-registry.

Re-run whenever shared/locales/*.yml, built-in UI strings, or the template library change.
"""
import argparse
import glob
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "fonts", "NotoSansSC-subset.otf")
SIBLING_TEMPLATES = os.path.join(os.path.dirname(ROOT),
                                "universal-shell-templates", "templates")
REGISTRY_MANIFEST = ("https://raw.githubusercontent.com/Jonnyan404/"
                     "universal-shell-templates/main/registry/manifests.json")


def collect_chars(templates_dirs, fetch_registry=True):
    pats = ["shared/locales/*.yml", "app-egui/src/main.rs"]
    files = [f for p in pats for f in glob.glob(os.path.join(ROOT, p))]

    texts = []
    for d in templates_dirs:
        matched = sorted(glob.glob(os.path.join(os.path.expanduser(d), "*.json")))
        if not matched:
            print(f"warn: no template json under {d}")
        files += matched

    if fetch_registry:
        import urllib.request
        try:
            with urllib.request.urlopen(REGISTRY_MANIFEST, timeout=15) as r:
                texts.append(r.read().decode("utf-8"))
            print(f"fetched template manifest: {REGISTRY_MANIFEST}")
        except Exception as e:  # offline / blocked: locales + local templates still cover the UI
            print(f"warn: manifest fetch failed ({e}); continuing without it")

    chars = set()
    for f in files:
        try:
            texts.append(open(f, encoding="utf-8").read())
        except OSError as e:
            print(f"skip {f}: {e}")
    for text in texts:
        for ch in text:
            if ('\u4e00' <= ch <= '\u9fff' or '\u3400' <= ch <= '\u4dbf'
                    or '\uf900' <= ch <= '\ufaff'
                    or '\u3000' <= ch <= '\u303f'
                    or '\uff00' <= ch <= '\uffef'):
                chars.add(ch)
    return len(files), chars


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--full-font", default="/tmp/NotoSansCJKsc-Regular.otf")
    ap.add_argument("--templates-dir", action="append", default=[],
                    help="模板库目录（可多次传入；默认 $US_TEMPLATES_DIR 或兄弟目录）")
    ap.add_argument("--no-fetch-registry", action="store_true",
                    help="不拉取官方 manifests.json（离线时用）")
    args = ap.parse_args()
    if not os.path.isfile(args.full_font):
        print(f"full font not found: {args.full_font}\nsee header for download URL")
        return 1

    from fontTools import subset

    dirs = args.templates_dir
    if not dirs:
        env_dir = os.environ.get("US_TEMPLATES_DIR", "").strip()
        dirs = [env_dir] if env_dir else [SIBLING_TEMPLATES]

    files_n, chars = collect_chars(dirs, fetch_registry=not args.no_fetch_registry)
    unicodes = sorted(ord(c) for c in chars)
    print(f"scanned {files_n} files, {len(chars)} unique chars")

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
