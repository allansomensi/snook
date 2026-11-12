"""Pictures for the Microsoft Store listing ("Store logos"): a 9:16 poster, a
1:1 box art and the 1:1 tile icons, from the game's emblem and font.

    python3 tools/store_art.py

Reads crates/client/assets/icon/snookered-1024.png (made by the
`export_icons` test) and writes store-art/. Needs Pillow.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
EMBLEM = ROOT / "crates/client/assets/icon/snookered-1024.png"
FONT = ROOT / "crates/client/assets/DejaVuSans-Bold.ttf"
OUT = ROOT / "store-art"
NAME = "SNOOKERED"

GOLD = (232, 191, 97)
FELT = (34, 120, 62)
DARK = (14, 9, 6)


def background(w, h):
    """Dark room, a pool of green cloth under the lamp in the middle."""
    img = Image.new("RGB", (w, h), DARK)
    glow = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(glow)
    r = int(min(w, h) * 0.62)
    cx, cy = w // 2, int(h * 0.44)
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=255)
    glow = glow.filter(ImageFilter.GaussianBlur(min(w, h) * 0.22))
    felt = Image.new("RGB", (w, h), FELT)
    return Image.composite(felt, img, glow.point(lambda v: int(v * 0.55)))


def emblem(size):
    return Image.open(EMBLEM).convert("RGBA").resize((size, size), Image.LANCZOS)


def title(img, text, cy, max_w, size):
    d = ImageDraw.Draw(img)
    font = ImageFont.truetype(str(FONT), size)
    while d.textlength(text, font=font) > max_w and size > 10:
        size -= 4
        font = ImageFont.truetype(str(FONT), size)
    w = d.textlength(text, font=font)
    x = (img.width - w) / 2
    # A soft shadow, then the gold name.
    shadow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).text((x + size * 0.04, cy + size * 0.05), text, font=font, fill=(0, 0, 0, 170))
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(size * 0.03)))
    ImageDraw.Draw(img).text((x, cy), text, font=font, fill=GOLD + (255,))


def art(w, h, emblem_size, emblem_y, title_y, title_size):
    img = background(w, h).convert("RGBA")
    e = emblem(emblem_size)
    # The emblem's shadow on the cloth.
    sh = Image.new("RGBA", img.size, (0, 0, 0, 0))
    mask = e.split()[3].point(lambda v: int(v * 0.6))
    sh.paste((0, 0, 0, 255), ((w - emblem_size) // 2, emblem_y + emblem_size // 30), mask)
    img.alpha_composite(sh.filter(ImageFilter.GaussianBlur(emblem_size // 25)))
    img.alpha_composite(e, ((w - emblem_size) // 2, emblem_y))
    title(img, NAME, title_y, w * 0.86, title_size)
    return img.convert("RGB")


def main():
    OUT.mkdir(exist_ok=True)
    # 9:16 poster (1440 x 2160) and 1:1 box art (2160 x 2160).
    art(1440, 2160, 1000, 520, 1640, 200).save(OUT / "poster-1440x2160.png", optimize=True)
    art(2160, 2160, 1150, 330, 1600, 260).save(OUT / "box-2160x2160.png", optimize=True)
    # Tile icons: the emblem alone, transparent around it.
    for size in (300, 150, 71):
        emblem(size).save(OUT / f"tile-{size}x{size}.png", optimize=True)
    print("written to", OUT)


if __name__ == "__main__":
    main()
