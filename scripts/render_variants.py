#!/usr/bin/env python3
"""Icon design variants (dock, tauri palette) for side-by-side review.

Saves 256px previews to assets/icons/variants/A..E-256.png.
A = current hexagon-plug (reference). Run from repo root:
    python3 scripts/render_variants.py
Stdlib only.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__))))
import render_icons as R  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "icons", "variants")

TOP, BOT = R.hx("#2B6BFF"), R.hx("#06B6D4")
WHITE = (255, 255, 255, 255)
AMBER = R.hx("#FBBF24")


def base_hexagon(cv, k):
    cx = cy = 512.0 * k
    RR = 400.0 * k
    y_lo = cy - RR * R.SQRT3_2
    y_hi = cy + RR * R.SQRT3_2
    for y in range(cv.h):
        yc = y + 0.5
        dx = R.hex_span(cx, RR, yc - cy)
        if dx < 0:
            continue
        t = (yc - y_lo) / (y_hi - y_lo)
        cv.fill_span(y, cx - dx, cx + dx, R.lerp(TOP, BOT, t))


def poly_spans(pts, y):
    xs = []
    n = len(pts)
    for i in range(n):
        x0, y0 = pts[i]
        x1, y1 = pts[(i + 1) % n]
        if (y0 <= y < y1) or (y1 <= y < y0):
            t = (y - y0) / (y1 - y0)
            xs.append(x0 + t * (x1 - x0))
    xs.sort()
    return [(xs[i], xs[i + 1]) for i in range(0, len(xs) - 1, 2)]


def fill_poly(cv, pts1024, k, color, y_lo=None, y_hi=None):
    pts = [(x * k, y * k) for x, y in pts1024]
    ys = [p[1] for p in pts]
    lo = int(max(0, min(ys))) if y_lo is None else y_lo
    hi = int(min(cv.h, max(ys))) if y_hi is None else y_hi
    for y in range(lo, hi):
        for s in poly_spans(pts, y + 0.5):
            cv.fill_span(y, s[0], s[1], color)


def render(size, painter):
    W = size * R.SS
    cv = R.Canvas(W, W)
    painter(cv, W / 1024.0)
    return R.downscale(cv, R.SS)


def paint_b_chevron(cv, k):
    """B: terminal chevron > + amber underscore (FINAL, optically centered)."""
    base_hexagon(cv, k)
    fill_poly(cv, [(395, 370), (565, 500), (395, 630),
                   (395, 545), (480, 500), (395, 455)], k, WHITE)
    for y in range(cv.h):
        yc = y + 0.5
        for s in R.rrect_spans(395 * k, 666 * k, 595 * k, 712 * k, 23 * k, yc):
            cv.fill_span(y, s[0], s[1], AMBER)


def paint_c_rings(cv, k):
    """C: double shell rings + amber core."""
    base_hexagon(cv, k)
    cx = cy = 512.0 * k
    for y in range(cv.h):
        yc = y + 0.5
        dy = yc - cy
        for outer, inner in ((400.0 * k, 352.0 * k), (258.0 * k, 218.0 * k)):
            dxo = R.hex_span(cx, outer, dy)
            if dxo < 0:
                continue
            segs = [(cx - dxo, cx + dxo)]
            dxi = R.hex_span(cx, inner, dy)
            if dxi >= 0:
                segs = R.subtract_spans(segs, [(cx - dxi, cx + dxi)])
            cv.fill_segments(y, segs, WHITE)
        for s in R.circle_span(cx, cy, 58 * k, yc):
            cv.fill_span(y, s[0], s[1], AMBER)


def paint_d_bolt(cv, k):
    """D: lightning bolt (launch)."""
    base_hexagon(cv, k)
    fill_poly(cv, [(580, 330), (420, 585), (505, 585),
                   (460, 730), (610, 495), (520, 495)], k, WHITE)


def paint_e_stack(cv, k):
    """E: three stacked app layers."""
    base_hexagon(cv, k)
    bars = [(322, 311, 702, 421, WHITE), (362, 457, 662, 567, AMBER),
            (402, 603, 622, 713, WHITE)]
    for y in range(cv.h):
        yc = y + 0.5
        for x0, y0, x1, y1, col in bars:
            for s in R.rrect_spans(x0 * k, y0 * k, x1 * k, y1 * k, 55 * k, yc):
                cv.fill_span(y, s[0], s[1], col)


def paint_f_play(cv, k):
    """F: play triangle (launch programs)."""
    base_hexagon(cv, k)
    fill_poly(cv, [(450, 420), (450, 640), (645, 530)], k, WHITE)
    for y in range(cv.h):
        yc = y + 0.5
        for s in R.circle_span(512 * k, 700 * k, 26 * k, yc):
            cv.fill_span(y, s[0], s[1], AMBER)


def paint_g_download(cv, k):
    """G: arrow into tray (fetch/install binaries)."""
    base_hexagon(cv, k)
    for y in range(cv.h):
        yc = y + 0.5
        for s in R.rrect_spans(490 * k, 360 * k, 534 * k, 545 * k, 22 * k, yc):
            cv.fill_span(y, s[0], s[1], WHITE)
    fill_poly(cv, [(458, 530), (566, 530), (512, 608)], k, WHITE)
    fill_poly(cv, [(400, 560), (400, 690), (624, 690), (624, 560),
                   (578, 560), (578, 644), (446, 644), (446, 560)], k, AMBER)


def paint_h_gear(cv, k):
    """H: gear (manage/configure programs)."""
    import math
    base_hexagon(cv, k)
    cx = cy = 512.0
    teeth = []
    for i in range(8):
        a = math.radians(i * 45.0)
        ca, sa = math.cos(a), math.sin(a)
        corners = []
        for lx, ly in ((-30, 108), (30, 108), (30, 200), (-30, 200)):
            corners.append((cx + lx * ca - ly * sa, cy + lx * sa + ly * ca))
        teeth.append(corners)
    lo = int(min(p[1] for t in teeth for p in t) * k) - 1
    hi = int(max(p[1] for t in teeth for p in t) * k) + 1
    for teeth_poly in teeth:
        fill_poly(cv, teeth_poly, k, WHITE,
                  y_lo=max(0, lo), y_hi=min(cv.h, hi))
    for y in range(cv.h):
        yc = y + 0.5
        for s in R.circle_span(cx * k, cy * k, 148 * k, yc):
            cv.fill_span(y, s[0], s[1], WHITE)
        for s in R.circle_span(cx * k, cy * k, 64 * k, yc):
            cv.fill_span(y, s[0], s[1], AMBER)


def paint_i_cube(cv, k):
    """I: package cube (binary artifacts)."""
    base_hexagon(cv, k)
    top = [(512, 372), (648, 452), (512, 532), (376, 452)]
    left = [(376, 452), (512, 532), (512, 692), (376, 612)]
    right = [(512, 532), (648, 452), (648, 612), (512, 692)]
    fill_poly(cv, left, k, WHITE)
    fill_poly(cv, right, k, (191, 219, 254, 255))
    fill_poly(cv, top, k, AMBER)


def main():
    os.makedirs(OUT, exist_ok=True)
    print("A (reference) ...", flush=True)
    R.save_png(R.render_dock(256, "#2B6BFF", "#06B6D4", "#FFFFFF", "#FBBF24"),
               os.path.join(OUT, "A-256.png"))
    for name, fn in [("B", paint_b_chevron), ("C", paint_c_rings),
                     ("D", paint_d_bolt), ("E", paint_e_stack),
                     ("F", paint_f_play), ("G", paint_g_download),
                     ("H", paint_h_gear), ("I", paint_i_cube)]:
        print(f"{name} ...", flush=True)
        R.save_png(render(256, fn), os.path.join(OUT, f"{name}-256.png"))
    print("done:", OUT)


if __name__ == "__main__":
    main()
