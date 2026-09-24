#!/usr/bin/env python3
"""Generate the xconsoler brand assets: SVG sources + the PNG icon set.

One geometry spec feeds two backends (an SVG writer and a PIL rasteriser), so
the vector mark and the installed icons can never drift apart.

    python3 scripts/gen-logo.py                # write assets/**
    python3 scripts/gen-logo.py --preview      # ASCII proof, write nothing

The mark is the product drawn as an icon: a rounded terminal tile holding the
long launcher bar (accent chevron + block cursor) over its candidate list.
"""

from __future__ import annotations

import argparse
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

# ── Palette (the kanagawa-wave theme of src/theme.rs, as brand colours) ─────
ACCENT = "#7e9cd8"     # crystalBlue: chevron, bar outline, selected label
WAVE = "#2d4f67"       # waveBlue2: the selected candidate row
CURSOR = "#c8c093"     # oldWhite: block cursor (and the TUI cursor ink)
TEXT = "#dcdcdc"       # fujiWhite: typed text, wordmark
MUTED = "#727169"      # fujiGray: dimmed chips
BORDER = "#38383f"     # divider chrome: the unselected candidate rows
INK_TOP = "#26262f"    # sumiInk, top-lit
INK_BOTTOM = "#16161d"
BAR_TOP = "#21212a"
BAR_BOTTOM = "#1a1a22"

CANVAS = 512           # design space; every coordinate below lives in it
SUPERSAMPLE = 4        # raster anti-aliasing factor
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
MINI_BELOW = 128       # sizes under this get the simplified mark

BRAND = "xconsoler"
FONT_SIZE = 156
MARK_PX = 320
PAD = 32
GAP = 44
FONT_PATHS = (
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationMono-Bold.ttf",
    "/usr/share/fonts/truetype/freefont/FreeMonoBold.ttf",
)
FONT_STACK = "DejaVu Sans Mono, Liberation Mono, ui-monospace, monospace"


# ── Geometry spec ──────────────────────────────────────────────────────────
def tile(sw=10, sop=0.35):
    """The rounded terminal tile both marks sit on."""
    pad = 12
    return {"k": "rrect", "x": pad, "y": pad, "w": CANVAS - 2 * pad,
            "h": CANVAS - 2 * pad, "r": 118,
            "fill": ("grad", INK_TOP, INK_BOTTOM),
            "stroke": ACCENT, "sw": sw, "sop": sop}


def detail_prims():
    """Full mark (>= MINI_BELOW px): tile + launcher bar + candidate list."""
    p = [tile(),
         {"k": "glow", "cx": 256, "cy": 146, "rx": 168, "ry": 92,
          "fill": ACCENT, "op": 0.26},
         # the long launcher bar: chevron, block cursor, typed text
         {"k": "rrect", "x": 68, "y": 104, "w": 376, "h": 84, "r": 42,
          "fill": ("grad", BAR_TOP, BAR_BOTTOM), "stroke": ACCENT, "sw": 8,
          "sop": 0.6},
         {"k": "poly", "pts": ((112, 124), (148, 146), (112, 168)), "sw": 20,
          "fill": ACCENT},
         {"k": "rrect", "x": 172, "y": 124, "w": 22, "h": 44, "r": 6,
          "fill": CURSOR},
         {"k": "rrect", "x": 214, "y": 138, "w": 164, "h": 16, "r": 8,
          "fill": TEXT, "op": 0.75}]
    rows = ((216, WAVE, 1.00), (284, BORDER, 0.90), (352, BORDER, 0.55))
    for y, fill, op in rows:
        p.append({"k": "rrect", "x": 68, "y": y, "w": 376, "h": 56, "r": 28,
                  "fill": fill, "op": op})
    chips = ((234, 96, ACCENT, 1.00, 204, 150, CURSOR, 0.55),
             (302, 84, TEXT, 0.55, 188, 132, MUTED, 0.95),
             (370, 76, TEXT, 0.35, 180, 118, MUTED, 0.65))
    for cy, lw, lc, lo, cx, cw, cc, co in chips:
        p.append({"k": "rrect", "x": 92, "y": cy, "w": lw, "h": 20, "r": 10,
                  "fill": lc, "op": lo})
        p.append({"k": "rrect", "x": cx, "y": cy, "w": cw, "h": 20, "r": 10,
                  "fill": cc, "op": co})
    return p


def mini_prims():
    """Simplified mark (< MINI_BELOW px): tile + chevron + cursor only."""
    return [tile(sw=14, sop=0.5),
            {"k": "glow", "cx": 256, "cy": 256, "rx": 176, "ry": 150,
             "fill": ACCENT, "op": 0.22},
            {"k": "poly", "pts": ((140, 168), (280, 256), (140, 344)),
             "sw": 68, "fill": ACCENT},
            {"k": "rrect", "x": 330, "y": 196, "w": 72, "h": 120, "r": 16,
             "fill": CURSOR}]


def prims_for(size):
    return mini_prims() if size < MINI_BELOW else detail_prims()


# ── Backend 1: SVG ─────────────────────────────────────────────────────────
def svg_defs(prims, tag):
    """Gradient definitions for this primitive list (`tag` keeps ids unique)."""
    out = []
    for i, p in enumerate(prims):
        if p["k"] == "glow":
            out.append(
                f'<radialGradient id="glow{tag}{i}">'
                f'<stop offset="0" stop-color="{p["fill"]}" stop-opacity="{p["op"]}"/>'
                f'<stop offset="1" stop-color="{p["fill"]}" stop-opacity="0"/>'
                f'</radialGradient>')
        elif isinstance(p.get("fill"), tuple):
            top, bottom = p["fill"][1], p["fill"][2]
            out.append(
                f'<linearGradient id="grad{tag}{i}" x1="0" y1="0" x2="0" y2="1">'
                f'<stop offset="0" stop-color="{top}"/>'
                f'<stop offset="1" stop-color="{bottom}"/></linearGradient>')
    return "".join(out)


def svg_body(prims, tag):
    out = []
    for i, p in enumerate(prims):
        op = f' opacity="{p["op"]}"' if p.get("op", 1.0) < 1.0 else ""
        if p["k"] == "glow":
            out.append(f'<ellipse cx="{p["cx"]}" cy="{p["cy"]}" rx="{p["rx"]}" '
                       f'ry="{p["ry"]}" fill="url(#glow{tag}{i})"/>')
        elif p["k"] == "poly":
            pts = " ".join(f"{x},{y}" for x, y in p["pts"])
            out.append(f'<polyline points="{pts}" fill="none" stroke="{p["fill"]}" '
                       f'stroke-width="{p["sw"]}" stroke-linecap="round" '
                       f'stroke-linejoin="round"{op}/>')
        else:
            fill = (f'url(#grad{tag}{i})' if isinstance(p.get("fill"), tuple)
                    else p.get("fill", "none"))
            stroke = ""
            if p.get("sw") and p.get("stroke"):
                stroke = (f' stroke="{p["stroke"]}" stroke-width="{p["sw"]}" '
                          f'stroke-opacity="{p.get("sop", 1.0)}"')
            out.append(f'<rect x="{p["x"]}" y="{p["y"]}" width="{p["w"]}" '
                       f'height="{p["h"]}" rx="{p["r"]}" fill="{fill}"'
                       f'{stroke}{op}/>')
    return "".join(out)


def icon_svg():
    prims = detail_prims()
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CANVAS} '
            f'{CANVAS}" width="{CANVAS}" height="{CANVAS}" role="img" '
            f'aria-label="{BRAND} icon"><defs>{svg_defs(prims, "")}</defs>'
            f'{svg_body(prims, "")}</svg>\n')


def logo_svg(font_path):
    """Mark + wordmark lockup; `textLength` pins the layout across renderers."""
    prims = detail_prims()
    font = mono_font(FONT_SIZE, font_path)
    width = text_metrics(font)
    total = PAD + MARK_PX + GAP + width["text"] + PAD
    height = MARK_PX + 2 * PAD
    scale = MARK_PX / CANVAS
    text_x = PAD + MARK_PX + GAP
    base_y = baseline_y(height, font, width)
    text = (f'<text x="{text_x}" y="{base_y:.1f}" font-family="{FONT_STACK}" '
            f'font-size="{FONT_SIZE}" font-weight="700" '
            f'textLength="{width["text"]:.1f}" lengthAdjust="spacing">'
            f'<tspan fill="{ACCENT}">{BRAND[0]}</tspan>'
            f'<tspan fill="{TEXT}">{BRAND[1:]}</tspan></text>')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total:.1f} '
            f'{height}" width="{total:.1f}" height="{height}" role="img" '
            f'aria-label="{BRAND}"><defs>{svg_defs(prims, "l")}</defs>'
            f'<g transform="translate({PAD},{PAD}) scale({scale})">'
            f'{svg_body(prims, "l")}</g>{text}</svg>\n')


# ── Backend 2: raster ──────────────────────────────────────────────────────
def hexa(color, op=1.0):
    c = color.lstrip("#")
    r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4))
    return (r, g, b, int(round(255 * op)))


def rrect_layer(p, size, k):
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    x0, y0 = p["x"] * k, p["y"] * k
    w, h, r = p["w"] * k, p["h"] * k, p["r"] * k
    fill = p.get("fill")
    if isinstance(fill, tuple):  # vertical gradient, masked to the rounded rect
        bw, bh = max(int(round(w)), 1), max(int(round(h)), 1)
        patch = gradient_patch(bw, bh, fill[1], fill[2], p.get("op", 1.0))
        mask = Image.new("L", (bw, bh), 0)
        ImageDraw.Draw(mask).rounded_rectangle((0, 0, bw - 1, bh - 1),
                                               radius=r, fill=255)
        layer.paste(patch, (int(round(x0)), int(round(y0))), mask)
    elif fill:
        ImageDraw.Draw(layer).rounded_rectangle((x0, y0, x0 + w, y0 + h),
                                                radius=r,
                                                fill=hexa(fill, p.get("op", 1.0)))
    sw = p.get("sw", 0) * k
    if sw and p.get("stroke"):
        # SVG straddles the path with a stroke; PIL paints inside the box, so
        # grow the box (and its radius) by half the width to get the same edge.
        half = sw / 2
        ImageDraw.Draw(layer).rounded_rectangle(
            (x0 - half, y0 - half, x0 + w + half, y0 + h + half),
            radius=r + half, outline=hexa(p["stroke"], p.get("sop", 1.0)),
            width=max(int(round(sw)), 1))
    return layer


def gradient_patch(w, h, top, bottom, op):
    col = Image.new("RGB", (1, max(h, 1)))
    px = col.load()
    t0, t1 = hexa(top), hexa(bottom)
    for y in range(h):
        t = y / (h - 1) if h > 1 else 0.0
        px[0, y] = tuple(round(t0[i] + (t1[i] - t0[i]) * t) for i in range(3))
    patch = col.resize((w, h)).convert("RGBA")
    patch.putalpha(Image.new("L", (w, h), int(round(255 * op))))
    return patch


def poly_layer(p, size, k):
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    pts = [(x * k, y * k) for x, y in p["pts"]]
    sw = max(p["sw"] * k, 1)
    color = hexa(p["fill"], p.get("op", 1.0))
    d.line(pts, fill=color, width=int(round(sw)), joint="curve")
    cap = sw / 2
    for x, y in (pts[0], pts[-1]):  # round caps at both ends
        d.ellipse((x - cap, y - cap, x + cap, y + cap), fill=color)
    return layer


def glow_layer(p, size, k):
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    cx, cy, rx, ry = p["cx"] * k, p["cy"] * k, p["rx"] * k, p["ry"] * k
    ImageDraw.Draw(layer).ellipse((cx - rx, cy - ry, cx + rx, cy + ry),
                                  fill=hexa(p["fill"], p.get("op", 1.0)))
    return layer.filter(ImageFilter.GaussianBlur(min(rx, ry) * 0.45))


def layer_for(p, size, k):
    if p["k"] == "glow":
        return glow_layer(p, size, k)
    if p["k"] == "poly":
        return poly_layer(p, size, k)
    return rrect_layer(p, size, k)


def render_mark(prims, size, canvas=CANVAS):
    s = size * SUPERSAMPLE
    k = s / canvas
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    for p in prims:
        img = Image.alpha_composite(img, layer_for(p, img.size, k))
    return img.resize((size, size), Image.LANCZOS)


def mono_font(size, font_path):
    for path in ((font_path,) if font_path else FONT_PATHS):
        if path and os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default(size)


def text_metrics(font):
    """Ink box + ascent, shared by the SVG and PNG lockups so they align."""
    left, top, right, bottom = font.getbbox(BRAND)
    ascent, _descent = font.getmetrics()
    return {"text": font.getlength(BRAND), "x": font.getlength(BRAND[0]),
            "top": top, "bottom": bottom, "ascent": ascent, "left": left}


def baseline_y(height, font, m):
    """SVG baseline that centres the ink exactly like the raster lockup."""
    return height / 2 - (m["top"] + m["bottom"]) / 2 + m["ascent"]


def render_logo(font_path):
    """Lockup raster: mark on the left, two-tone wordmark on the right."""
    font = mono_font(FONT_SIZE, font_path)
    m = text_metrics(font)
    total = int(round(PAD + MARK_PX + GAP + m["text"] + PAD))
    height = MARK_PX + 2 * PAD
    img = Image.new("RGBA", (total, height), (0, 0, 0, 0))
    mark = render_mark(detail_prims(), MARK_PX)
    img.paste(mark, (PAD, PAD), mark)
    d = ImageDraw.Draw(img)
    y = int(round(height / 2 - (m["top"] + m["bottom"]) / 2))
    x = PAD + MARK_PX + GAP - m["left"]
    d.text((x, y), BRAND[0], font=font, fill=hexa(ACCENT))
    d.text((x + m["x"], y), BRAND[1:], font=font, fill=hexa(TEXT))
    return img


# ── ASCII proof (so the mark can be reviewed without an image viewer) ──────
RAMP = " .:-=+*#%@"


def ascii_art(img, cols=72):
    """Luminance proof of a rendered image (terminal cells are ~2:1)."""
    over = Image.new("RGBA", img.size, (8, 12, 18, 255))
    flat = Image.alpha_composite(over, img).convert("L")
    rows = max(1, round(cols * img.height / img.width / 2.1))
    small = flat.resize((cols, rows), Image.LANCZOS)
    px = small.load()
    return "\n".join(
        "".join(RAMP[min(len(RAMP) - 1, px[c, r] * len(RAMP) // 256)]
                for c in range(cols)) for r in range(rows))


# ── Entry point ────────────────────────────────────────────────────────────
def write(path, payload, binary=False):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    if binary:
        with open(path, "wb") as fh:
            payload.save(fh, optimize=True)
    else:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(payload)
    return path


def generate(out_dir, font_path):
    written = [write(os.path.join(out_dir, "icon.svg"), icon_svg()),
               write(os.path.join(out_dir, "logo.svg"), logo_svg(font_path)),
               write(os.path.join(out_dir, "png", "logo.png"),
                     render_logo(font_path), binary=True)]
    for size in SIZES:
        written.append(write(os.path.join(out_dir, "png", f"{BRAND}-{size}.png"),
                             render_mark(prims_for(size), size), binary=True))
    return written


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default="assets", help="asset output directory")
    ap.add_argument("--font", default=None, help="mono TTF for the wordmark")
    ap.add_argument("--preview", action="store_true",
                    help="print ASCII proofs instead of writing files")
    args = ap.parse_args()
    if args.preview:
        for size in (512, 48, 16):
            print(f"── mark @ {size}px " + "─" * 40)
            print(ascii_art(render_mark(prims_for(size), size),
                            cols=48 if size < 64 else 72))
        print("── lockup " + "─" * 46)
        print(ascii_art(render_logo(args.font), cols=96))
        return
    for path in generate(args.out, args.font):
        print("wrote", path)


if __name__ == "__main__":
    main()
