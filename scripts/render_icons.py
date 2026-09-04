#!/usr/bin/env python3
"""Render universal-shell icon masters to PNG / icns / ico / raw RGBA.

Reads geometry from assets/icons/*.svg (human-editable masters); this script
is the authoritative rasterizer so dock + tray stay in sync.

Stdlib only. Run from repo root:
    python3 scripts/render_icons.py

Outputs:
  assets/icons/png/*.png        1024 masters + previews
  assets/icons/rgba/*.rgba      raw RGBA for egui include_bytes!
  app-tauri/src-tauri/icons/    bundle icons (tauri variant) + tray-tauri.png
"""
import math
import os
import struct
import subprocess
import sys
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets", "icons")
PNGDIR = os.path.join(ASSETS, "png")
RGBADIR = os.path.join(ASSETS, "rgba")
TAURI_ICONS = os.path.join(ROOT, "app-tauri", "src-tauri", "icons")

SQRT3_2 = math.sqrt(3) / 2.0
SS = 2  # supersample factor for edge AA


def hx(s):
    s = s.lstrip("#")
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16), 255)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t + 0.5) for i in range(4))


class Canvas:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = bytearray(w * h * 4)

    def blend_px(self, x, y, r, g, b, a, cov):
        if cov <= 0.0 or x < 0 or y < 0 or x >= self.w or y >= self.h:
            return
        o = (y * self.w + x) * 4
        px = self.px
        if cov >= 1.0 and a >= 255:
            px[o:o + 4] = bytes((r, g, b, 255))
            return
        alpha = (a / 255.0) * cov
        inv = 1.0 - alpha
        px[o] = int(r * alpha + px[o] * inv + 0.5)
        px[o + 1] = int(g * alpha + px[o + 1] * inv + 0.5)
        px[o + 2] = int(b * alpha + px[o + 2] * inv + 0.5)
        px[o + 3] = int(255 * (alpha + (px[o + 3] / 255.0) * inv) + 0.5)

    def fill_span(self, y, x0, x1, color):
        """Fill fractional horizontal span on row y with coverage AA edges."""
        if y < 0 or y >= self.h or x1 <= 0 or x0 >= self.w:
            return
        x0 = max(x0, 0.0)
        x1 = min(x1, float(self.w))
        if x1 <= x0:
            return
        r, g, b, a = color
        lx = math.floor(x0)
        rx = math.floor(x1 - 1e-9)
        if lx == rx:
            self.blend_px(lx, y, r, g, b, a, x1 - x0)
            return
        self.blend_px(lx, y, r, g, b, a, (lx + 1) - x0)
        if a == 255:
            o = (y * self.w + lx + 1) * 4
            self.px[o:o + (rx - lx - 1) * 4] = bytes((r, g, b, a)) * max(0, rx - lx - 1)
        else:
            for x in range(lx + 1, rx):
                self.blend_px(x, y, r, g, b, a, 1.0)
        self.blend_px(rx, y, r, g, b, a, x1 - rx)

    def fill_segments(self, y, segs, color):
        for x0, x1 in segs:
            self.fill_span(y, x0, x1, color)


def hex_span(cx, r, dy):
    """Half-width of flat-top hexagon (center cx, radius r) at |dy|."""
    ady = abs(dy)
    if ady > r * SQRT3_2:
        return -1.0
    return min(r, 2.0 * (r - SQRT3_2 * ady))


def rrect_spans(x0, y0, x1, y1, rad, y):
    """Span(s) of rounded rect on row-center y; [] if none."""
    if y < y0 or y > y1:
        return []
    if y < y0 + rad:
        dx = math.sqrt(max(0.0, rad * rad - (y - (y0 + rad)) ** 2))
        return [(x0 + rad - dx, x1 - rad + dx)]
    if y > y1 - rad:
        dx = math.sqrt(max(0.0, rad * rad - (y - (y1 - rad)) ** 2))
        return [(x0 + rad - dx, x1 - rad + dx)]
    return [(x0, x1)]


def circle_span(cx, cy, r, y):
    dy = abs(y - cy)
    if dy > r:
        return []
    dx = math.sqrt(r * r - dy * dy)
    return [(cx - dx, cx + dx)]


def merge_spans(spans):
    spans = sorted(spans)
    out = []
    for s in spans:
        if out and s[0] <= out[-1][1]:
            out[-1] = (out[-1][0], max(out[-1][1], s[1]))
        else:
            out.append(s)
    return out


def subtract_spans(base, holes):
    out = []
    for b0, b1 in base:
        cur = [(b0, b1)]
        for h0, h1 in holes:
            nxt = []
            for c0, c1 in cur:
                if h1 <= c0 or h0 >= c1:
                    nxt.append((c0, c1))
                else:
                    if h0 > c0:
                        nxt.append((c0, h0))
                    if h1 < c1:
                        nxt.append((h1, c1))
            cur = nxt
        out.extend(cur)
    return out


# ---- dock: gradient hexagon + plug + LED -----------------------------------
def render_dock(size, top, bottom, plug_c, led_c):
    W = size * SS
    cv = Canvas(W, W)
    k = W / 1024.0
    cx = cy = 512.0 * k
    R = 400.0 * k
    top_c, bot_c = hx(top), hx(bottom)
    y_lo = cy - R * SQRT3_2
    y_hi = cy + R * SQRT3_2
    # plug geometry (1024 units)
    prongs = [(455, 392, 501, 560, 23), (523, 392, 569, 560, 23)]
    body = (417, 545, 607, 725, 44)
    led = (512, 635, 32)
    plug = hx(plug_c)
    ledcol = hx(led_c)
    for y in range(W):
        yc = y + 0.5
        dx = hex_span(cx, R, yc - cy)
        if dx < 0:
            continue
        t = (yc - y_lo) / (y_hi - y_lo)
        cv.fill_span(y, cx - dx, cx + dx, lerp(top_c, bot_c, t))
        if 392 * k <= yc <= 725 * k:
            holes = []
            for (x0, y0, x1, y1, rad) in prongs:
                holes += rrect_spans(x0 * k, y0 * k, x1 * k, y1 * k, rad * k, yc)
            holes += rrect_spans(body[0] * k, body[1] * k, body[2] * k,
                                 body[3] * k, body[4] * k, yc)
            for s in merge_spans(holes):
                cv.fill_span(y, s[0], s[1], plug)
        if (635 - 32) * k <= yc <= (635 + 32) * k:
            for s in circle_span(512 * k, 635 * k, 32 * k, yc):
                cv.fill_span(y, s[0], s[1], ledcol)
    return downscale(cv, SS)


# ---- tray: template glyphs, black + white keyline ---------------------------
TRAY_R = 112.0
TRAY_C = 128.0


def render_tray(size, variant):
    W = size * SS
    cv = Canvas(W, W)
    k = W / 256.0
    cx = cy = TRAY_C * k
    R = TRAY_R * k
    black = (0, 0, 0, 255)
    white = (255, 255, 255, 255)
    y_lo = cy - (R + 6 * k) * SQRT3_2
    y_hi = cy + (R + 6 * k) * SQRT3_2
    if variant == "tauri":
        holes_src = [(104, 64, 122, 132, 9), (134, 64, 152, 132, 9),
                     (88, 126, 168, 196, 16)]
    else:
        holes_src = []  # egui draws ring + solid plug instead
    for y in range(W):
        yc = y + 0.5
        if not (y_lo <= yc <= y_hi):
            continue
        # 1) white keyline ring (outer R+6, inner R-1)
        dxo = hex_span(cx, R + 6 * k, yc - cy)
        dxi = hex_span(cx, R - 1 * k, yc - cy)
        if dxo >= 0:
            segs = [(cx - dxo, cx + dxo)]
            if dxi >= 0:
                segs = subtract_spans(segs, [(cx - dxi, cx + dxi)])
            cv.fill_segments(y, segs, white)
        # 2) black body
        dx = hex_span(cx, R, yc - cy)
        if dx < 0:
            continue
        if variant == "tauri":
            holes = []
            for (x0, y0, x1, y1, rad) in holes_src:
                holes += rrect_spans(x0 * k, y0 * k, x1 * k, y1 * k, rad * k, yc)
            segs = subtract_spans([(cx - dx, cx + dx)], merge_spans(holes))
            cv.fill_segments(y, segs, black)
        else:
            dxi2 = hex_span(cx, 88 * k, yc - cy)
            segs = [(cx - dx, cx + dx)]
            if dxi2 >= 0:
                segs = subtract_spans(segs, [(cx - dxi2, cx + dxi2)])
            cv.fill_segments(y, segs, black)
            plug_src = [(106, 50, 122, 134, 8), (134, 50, 150, 134, 8),
                        (92, 130, 164, 190, 12)]
            ps = []
            for (x0, y0, x1, y1, rad) in plug_src:
                ps += rrect_spans(x0 * k, y0 * k, x1 * k, y1 * k, rad * k, yc)
            for s in merge_spans(ps):
                cv.fill_span(y, s[0], s[1], black)
    return downscale(cv, SS)


def downscale(src, factor):
    w, h = src.w // factor, src.h // factor
    dst = Canvas(w, h)
    sp, dp = src.px, dst.px
    sw = src.w
    n = factor * factor
    for y in range(h):
        for x in range(w):
            rs = gs = bs = asum = 0
            for dy in range(factor):
                base = ((y * factor + dy) * sw + x * factor) * 4
                for dxi in range(factor):
                    o = base + dxi * 4
                    a = sp[o + 3]
                    asum += a
                    rs += sp[o] * a
                    gs += sp[o + 1] * a
                    bs += sp[o + 2] * a
            if asum == 0:
                continue
            o = (y * w + x) * 4
            dp[o] = round(rs / asum)
            dp[o + 1] = round(gs / asum)
            dp[o + 2] = round(bs / asum)
            dp[o + 3] = round(asum / n)
    return dst


def save_png(cv, path):
    w, h = cv.w, cv.h
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        raw += cv.px[y * w * 4:(y + 1) * w * 4]
    comp = zlib.compress(bytes(raw), 9)

    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        return c + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", comp)
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)


def save_rgba(cv, path):
    with open(path, "wb") as f:
        f.write(cv.px)


def save_ico(pngs, path):
    """pngs: list of (size, bytes). Writes Vista-style PNG-compressed ICO."""
    n = len(pngs)
    header = struct.pack("<HHH", 0, 1, n)
    offset = 6 + 16 * n
    dirs, blobs = b"", b""
    for size, data in pngs:
        w = 0 if size >= 256 else size
        dirs += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        blobs += data
    with open(path, "wb") as f:
        f.write(header + dirs + blobs)


def read_png(path):
    with open(path, "rb") as f:
        return f.read()


def halve(cv):
    return downscale(cv, 2)


def main():
    os.makedirs(PNGDIR, exist_ok=True)
    os.makedirs(RGBADIR, exist_ok=True)

    print("render dock-tauri @1024 ...", flush=True)
    dock_t = render_dock(1024, "#2B6BFF", "#06B6D4", "#FFFFFF", "#FBBF24")
    print("render dock-egui @1024 ...", flush=True)
    dock_e = render_dock(1024, "#1E293B", "#065F46", "#4ADE80", "#FFFFFF")

    save_png(dock_t, os.path.join(PNGDIR, "dock-tauri-1024.png"))
    save_png(dock_e, os.path.join(PNGDIR, "dock-egui-1024.png"))

    # tauri bundle set derived by halving (sharper + consistent)
    sizes = {"icon.png": 512, "128x128@2x.png": 256, "128x128.png": 128,
             "32x32.png": 32}
    cur, cur_size = dock_t, 1024
    chain = {}
    while cur_size >= 16:
        chain[cur_size] = cur
        if cur_size == 16:
            break
        cur = halve(cur)
        cur_size //= 2
    for name, sz in sizes.items():
        save_png(chain[sz], os.path.join(TAURI_ICONS, name))
    # iconset for iconutil
    iconset = os.path.join(TAURI_ICONS, "tmp.iconset")
    os.makedirs(iconset, exist_ok=True)
    pairs = [("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
             ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
             ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
             ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
             ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024)]
    for name, sz in pairs:
        save_png(chain[sz], os.path.join(iconset, name))
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o",
                    os.path.join(TAURI_ICONS, "icon.icns")], check=True)
    for f in os.listdir(iconset):
        os.remove(os.path.join(iconset, f))
    os.rmdir(iconset)
    # ico (PNG-compressed entries; 64 covers the classic-48 slot, Vista+ scales)
    ico_blobs = []
    for sz in (16, 32, 64, 256):
        tmp = os.path.join(PNGDIR, f"_ico_{sz}.png")
        save_png(chain[sz], tmp)
        ico_blobs.append((sz, read_png(tmp)))
        os.remove(tmp)
    save_ico(ico_blobs, os.path.join(TAURI_ICONS, "icon.ico"))

    # egui runtime assets: 256 dock (png preview + raw rgba for include_bytes!)
    ecur, esz = dock_e, 1024
    while esz > 256:
        ecur = halve(ecur)
        esz //= 2
    save_png(ecur, os.path.join(PNGDIR, "dock-egui-256.png"))
    save_rgba(ecur, os.path.join(RGBADIR, "dock-egui-256.rgba"))

    # tray
    print("render tray ...", flush=True)
    tray_t = render_tray(32, "tauri")
    tray_e = render_tray(32, "egui")
    save_png(tray_t, os.path.join(TAURI_ICONS, "tray-tauri.png"))
    save_png(tray_e, os.path.join(PNGDIR, "tray-egui-32.png"))
    save_rgba(tray_t, os.path.join(RGBADIR, "tray-tauri-32.rgba"))
    save_rgba(tray_e, os.path.join(RGBADIR, "tray-egui-32.rgba"))
    print("done.")


if __name__ == "__main__":
    sys.exit(main())
