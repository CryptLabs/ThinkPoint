#!/usr/bin/env python3
"""Regenerate the ThinkPoint logo assets from the two SVG sources.

    python3 assets/build.py

Needs cairosvg and Pillow, and DejaVu Sans for the wordmark text.

Two things here are less obvious than they look. Text positions are measured
from the actual font file rather than eyeballed, so the red nub lands exactly
where the "o" of Point would have been at any size. And everything is rendered
several times larger than needed and downsampled, because cairosvg leaves
colour fringing on text drawn at its final size.
"""

import pathlib
import subprocess

from PIL import Image, ImageFont

HERE = pathlib.Path(__file__).parent
BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
REGULAR = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"

BG = "#14171a"
KEY = "#39414a"
INK_DARK_THEME = "#e8eaed"
INK_LIGHT_THEME = "#1b1f24"
MUTED = "#828b96"
CARD_BG = "#0d1117"

DEFS = """<defs>
    <radialGradient id="nub" cx="38%" cy="34%" r="72%">
      <stop offset="0%" stop-color="#f2564a"/>
      <stop offset="55%" stop-color="#d93a2f"/>
      <stop offset="100%" stop-color="#a8241c"/>
    </radialGradient>
    <linearGradient id="plate" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#21262d"/>
      <stop offset="100%" stop-color="#181c22"/>
    </linearGradient>
  </defs>"""

KEYS = (
    [(94, 174), (150, 174), (206, 174), (262, 174), (318, 174), (374, 174)]
    + [(108, 234), (164, 234), (304, 234), (360, 234)]
    + [(94, 294), (150, 294), (206, 294), (262, 294), (318, 294), (374, 294)]
)


def text_width(text: str, font_path: str, size: int) -> float:
    return ImageFont.truetype(font_path, size).getlength(text)


def mark(x: float, y: float, size: float) -> str:
    """The keyboard mark, scaled from its native 512 box to `size`."""
    k = size / 512.0

    def s(v):
        return round(v * k, 2)

    keys = "".join(
        f'<rect x="{s(kx)}" y="{s(ky)}" width="{s(44)}" height="{s(44)}" rx="{s(11)}"/>'
        for kx, ky in KEYS
    )
    return f"""<g transform="translate({round(x, 2)},{round(y, 2)})">
    <rect width="{s(512)}" height="{s(512)}" rx="{s(112)}" fill="{BG}"/>
    <rect x="{s(64)}" y="{s(136)}" width="{s(384)}" height="{s(240)}" rx="{s(30)}"
          fill="url(#plate)" stroke="#2b3138" stroke-width="{max(1, s(2))}"/>
    <g fill="{KEY}">{keys}</g>
    <circle cx="{s(256)}" cy="{s(256)}" r="{s(42)}" fill="{BG}"/>
    <circle cx="{s(256)}" cy="{s(256)}" r="{s(34)}" fill="url(#nub)"/>
  </g>"""


def render(src: str, out: str, width: int, supersample: int = 3) -> None:
    tmp = f"/tmp/_thinkpoint_{out}"
    subprocess.run(
        ["cairosvg", str(HERE / src), "-o", tmp, "--output-width", str(width * supersample)],
        check=True,
    )
    image = Image.open(tmp)
    image.resize((width, round(image.height / supersample)), Image.LANCZOS).save(HERE / out)


# ---- Wordmark ---------------------------------------------------------------

FONT_SIZE = 132
PRE, POST = "ThinkP", "int"
PRE_W = text_width(PRE, BOLD, FONT_SIZE)
O_W = text_width("o", BOLD, FONT_SIZE)
POST_W = text_width(POST, BOLD, FONT_SIZE)

MARK_SIZE = 200
GAP = 44
PAD = 48
TEXT_X = PAD + MARK_SIZE + GAP
WORD_W = round(TEXT_X + PRE_W + O_W + POST_W + PAD)
WORD_H = 296
BASELINE = 196


def wordmark_svg(ink: str) -> str:
    """GitHub serves READMEs on a light or a dark background, so the same
    geometry is emitted twice with different ink and chosen with <picture>."""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WORD_W} {WORD_H}"
     width="{WORD_W}" height="{WORD_H}" role="img" aria-label="ThinkPoint">
  <title>ThinkPoint</title>
  {DEFS}
  {mark(PAD, (WORD_H - MARK_SIZE) / 2, MARK_SIZE)}
  <text x="{TEXT_X}" y="{BASELINE}" font-family="DejaVu Sans, Verdana, sans-serif"
        font-size="{FONT_SIZE}" font-weight="bold" fill="{ink}">{PRE}</text>
  <circle cx="{round(TEXT_X + PRE_W + O_W / 2, 2)}" cy="{round(BASELINE - FONT_SIZE * 0.30, 2)}"
          r="{round(FONT_SIZE * 0.30, 2)}" fill="url(#nub)"/>
  <text x="{round(TEXT_X + PRE_W + O_W, 2)}" y="{BASELINE}" font-family="DejaVu Sans, Verdana, sans-serif"
        font-size="{FONT_SIZE}" font-weight="bold" fill="{ink}">{POST}</text>
</svg>
"""


# ---- Social preview ---------------------------------------------------------

CARD_W, CARD_H = 1280, 640
CARD_MARK = 208
TITLE_SIZE = 104
TAG_SIZE = 36
TAGLINE = "TrackPoint tuning, button maps and libinput"


def social_svg() -> str:
    title_pre = text_width(PRE, BOLD, TITLE_SIZE)
    title_o = text_width("o", BOLD, TITLE_SIZE)
    title_post = text_width(POST, BOLD, TITLE_SIZE)
    title_w = title_pre + title_o + title_post
    tag_w = text_width(TAGLINE, REGULAR, TAG_SIZE)

    gap = 56
    block = max(title_w, tag_w)
    group = CARD_MARK + gap + block
    # Catch an overlong tagline here rather than in the rendered card, where it
    # simply runs off the edge.
    assert group <= CARD_W - 120, f"lockup is {group:.0f}px, too wide for {CARD_W}px"

    mark_x = (CARD_W - group) / 2
    text_x = mark_x + CARD_MARK + gap
    centre = CARD_H / 2

    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {CARD_W} {CARD_H}"
     width="{CARD_W}" height="{CARD_H}" role="img" aria-label="ThinkPoint">
  <title>ThinkPoint</title>
  {DEFS}
  <rect width="{CARD_W}" height="{CARD_H}" fill="{CARD_BG}"/>
  {mark(mark_x, centre - CARD_MARK / 2, CARD_MARK)}
  <text x="{round(text_x, 2)}" y="{round(centre - 4, 2)}" font-family="DejaVu Sans, Verdana, sans-serif"
        font-size="{TITLE_SIZE}" font-weight="bold" fill="{INK_DARK_THEME}">{PRE}</text>
  <circle cx="{round(text_x + title_pre + title_o / 2, 2)}" cy="{round(centre - 4 - TITLE_SIZE * 0.30, 2)}"
          r="{round(TITLE_SIZE * 0.30, 2)}" fill="url(#nub)"/>
  <text x="{round(text_x + title_pre + title_o, 2)}" y="{round(centre - 4, 2)}"
        font-family="DejaVu Sans, Verdana, sans-serif"
        font-size="{TITLE_SIZE}" font-weight="bold" fill="{INK_DARK_THEME}">{POST}</text>
  <text x="{round(text_x, 2)}" y="{round(centre + 62, 2)}" font-family="DejaVu Sans, Verdana, sans-serif"
        font-size="{TAG_SIZE}" fill="{MUTED}">{TAGLINE}</text>
</svg>
"""


def main() -> None:
    (HERE / "wordmark-dark.svg").write_text(wordmark_svg(INK_DARK_THEME))
    (HERE / "wordmark-light.svg").write_text(wordmark_svg(INK_LIGHT_THEME))
    (HERE / "social-preview.svg").write_text(social_svg())

    # logo.svg keeps the full six-key rows; below 48px they turn to mush, so
    # the small sizes come from logo-small.svg instead.
    for size in (512, 256, 128, 64):
        render("logo.svg", f"logo-{size}.png", size)
    for size in (48, 32, 16):
        render("logo-small.svg", f"logo-{size}.png", size, supersample=4)

    render("wordmark-dark.svg", "wordmark-dark.png", 760)
    render("wordmark-light.svg", "wordmark-light.png", 760)
    render("social-preview.svg", "social-preview.png", 1280)

    icons = [Image.open(HERE / f"logo-{s}.png").convert("RGBA") for s in (48, 32, 16)]
    icons[0].save(HERE / "favicon.ico", sizes=[(48, 48), (32, 32), (16, 16)])

    print("assets rebuilt")


if __name__ == "__main__":
    main()
