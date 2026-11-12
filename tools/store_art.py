"""Pictures for the Microsoft Store listing ("Store logos"): a 9:16 poster, a
1:1 box art and the 1:1 tile icons, from the game's emblem and font.

    python3 tools/store_art.py

Reads crates/client/assets/icon/snookered-1024.png (made by the
`export_icons` test) and store-art/scene-3840x2160.png, the pool hall drawn
by the game without its interface (a debug build):

    SNOOKERED_SCENE_ONLY=1 SNOOKERED_CAM="1.6,1.25,-1.4,-0.2,0.75,0" \
      snookered --window 3840x2160 --screenshot store-art/scene-3840x2160.png --frames 240

(with `focused_scene = false` and `quality = Ultra` in the settings, so the
whole room is lit). Writes store-art/. Needs Pillow.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
EMBLEM = ROOT / "crates/client/assets/icon/snookered-1024.png"
SCENE = ROOT / "store-art/scene-3840x2160.png"
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


def scene_crop(w, h, focus_x=0.5):
    """The pool hall, cropped to w:h around focus_x and scaled to w x h."""
    src = Image.open(SCENE).convert("RGBA")
    sw, sh = src.size
    if sw / sh > w / h:
        cw, ch = int(sh * w / h), sh
    else:
        cw, ch = sw, int(sw * h / w)
    x = min(max(int(sw * focus_x - cw / 2), 0), sw - cw)
    y = (sh - ch) // 2
    return src.crop((x, y, x + cw, y + ch)).resize((w, h), Image.LANCZOS)


def darken_top(img, strength=0.55):
    """A dark band at the top, so a title on it reads well."""
    h = img.height
    band = Image.new("L", img.size, 0)
    d = ImageDraw.Draw(band)
    for y in range(h // 2):
        d.line([(0, y), (img.width, y)], fill=int(255 * strength * (1 - y / (h / 2))))
    shade = Image.new("RGBA", img.size, (0, 0, 0, 255))
    img.paste(shade, (0, 0), band)
    return img


def main():
    OUT.mkdir(exist_ok=True)
    # 9:16 poster (1440 x 2160) and 1:1 box art (2160 x 2160).
    art(1440, 2160, 1000, 520, 1640, 200).save(OUT / "poster-1440x2160.png", optimize=True)
    art(2160, 2160, 1150, 330, 1600, 260).save(OUT / "box-2160x2160.png", optimize=True)
    # Tile icons: the emblem alone, transparent around it.
    for size in (300, 150, 71):
        emblem(size).save(OUT / f"tile-{size}x{size}.png", optimize=True)
    if SCENE.exists():
        # Windows and Xbox: the 16:9 "super hero" art, without the title.
        scene_crop(3840, 2160).convert("RGB").save(OUT / "hero-3840x2160.png", optimize=True)
        # Xbox only: key art and hero with the title (in the top three
        # quarters), and the featured promotional square without it.
        key = darken_top(scene_crop(584, 800, focus_x=0.35))
        title(key, NAME, 70, 584 * 0.86, 90)
        key.convert("RGB").save(OUT / "xbox-key-art-584x800.png", optimize=True)
        hero = darken_top(scene_crop(1920, 1080))
        title(hero, NAME, 90, 1920 * 0.6, 150)
        hero.convert("RGB").save(OUT / "xbox-hero-titled-1920x1080.png", optimize=True)
        scene_crop(1080, 1080, focus_x=0.35).convert("RGB").save(
            OUT / "xbox-promo-square-1080x1080.png", optimize=True
        )
    print("written to", OUT)


if __name__ == "__main__":
    main()
