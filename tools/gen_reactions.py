#!/usr/bin/env python3
"""Generates the reaction atlas (crates/client/assets/reactions.png) from the
pictures named in crates/proto/src/reactions.rs, in that order.

Requirements: `pip install pillow`, `rsvg-convert` (librsvg) and the Twemoji
SVGs (graphics CC-BY 4.0) unpacked somewhere:

    npm pack @twemoji/svg@15.0.0 && tar xzf twemoji-svg-15.0.0.tgz
    python3 tools/gen_reactions.py path/to/package
"""
import os
import re
import subprocess
import sys
import tempfile

from PIL import Image

SIZE, COLS = 72, 8
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main(svg_dir):
    src = open(os.path.join(ROOT, "crates/proto/src/reactions.rs")).read()
    arts = re.findall(r'r\("[a-z_]+", "([0-9a-f]+)"', src)
    rows = (len(arts) + COLS - 1) // COLS
    atlas = Image.new("RGBA", (COLS * SIZE, rows * SIZE), (0, 0, 0, 0))
    with tempfile.TemporaryDirectory() as tmp:
        for i, art in enumerate(arts):
            png = os.path.join(tmp, art + ".png")
            subprocess.run(
                ["rsvg-convert", "-w", str(SIZE), "-h", str(SIZE), "-o", png,
                 os.path.join(svg_dir, art + ".svg")],
                check=True,
            )
            atlas.paste(Image.open(png).convert("RGBA"), ((i % COLS) * SIZE, (i // COLS) * SIZE))
    out = os.path.join(ROOT, "crates/client/assets/reactions.png")
    atlas.save(out, optimize=True)
    print(f"{len(arts)} reactions -> {out}")


if __name__ == "__main__":
    main(sys.argv[1])
