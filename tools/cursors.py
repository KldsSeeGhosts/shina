"""Draw Shina's closed-hand cursor (CSS `grabbing`), shown while the effort slider is held.

Windows has no native hand-grab cursor, and winit maps `Grabbing` to the four-way move arrow, so
Shina ships its own. It is a 64x64 PNG: Rune draws cursor images at 2x, so it is a 32 pt
cursor. Run with `py -I tools/cursors.py assets/cursors`.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

SS = 16  # supersampling: draw in 512 px, downsample to 64 px (2 px per point)
UNIT = 2 * SS  # pixels per point at the supersampled size
OUTLINE = 1.4  # points of dark edge around the white hand


def capsule(draw, a, b, radius, fill):
    (x1, y1), (x2, y2) = a, b
    r = radius * UNIT
    draw.line([(x1 * UNIT, y1 * UNIT), (x2 * UNIT, y2 * UNIT)], fill=fill, width=int(2 * r))
    for x, y in (a, b):
        draw.ellipse([x * UNIT - r, y * UNIT - r, x * UNIT + r, y * UNIT + r], fill=fill)


def palm(draw, box, radius, grow, fill):
    x1, y1, x2, y2 = box
    g = grow
    draw.rounded_rectangle(
        [(x1 - g) * UNIT, (y1 - g) * UNIT, (x2 + g) * UNIT, (y2 + g) * UNIT],
        radius=(radius + g) * UNIT,
        fill=fill,
    )


def hand(fingers, thumb, palm_box):
    """fingers: (x, top, bottom) capsules; thumb: (start, end); all in points of a 32 pt box."""
    image = Image.new("RGBA", (32 * UNIT, 32 * UNIT), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    finger_r = 2.0
    for grow, colour in ((OUTLINE, (24, 24, 24, 255)), (0.0, (255, 255, 255, 255))):
        for x, top, bottom in fingers:
            capsule(draw, (x, top), (x, bottom), finger_r + grow, colour)
        capsule(draw, thumb[0], thumb[1], finger_r + grow, colour)
        palm(draw, palm_box, 4.5, grow, colour)
    # Seams between fingers, so they read as separate digits.
    for (x, top, bottom), (nx, _, _) in zip(fingers, fingers[1:]):
        mid = (x + nx) / 2
        y_end = min(bottom, palm_box[1] + 3.0)
        draw.line(
            [(mid * UNIT, (top + 2.5) * UNIT), (mid * UNIT, y_end * UNIT)],
            fill=(24, 24, 24, 255),
            width=int(0.9 * UNIT),
        )
    return image.resize((64, 64), Image.LANCZOS)


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    palm_box = (9.0, 14.5, 25.0, 28.0)
    closed_hand = hand(
        fingers=[(11.0, 12.0, 16.0), (15.0, 11.0, 16.0), (19.0, 11.0, 16.0), (23.0, 12.0, 16.0)],
        thumb=((10.5, 21.5), (7.0, 17.0)),
        palm_box=palm_box,
    )
    closed_hand.save(out / "grabbing.png", optimize=True)
    preview = Image.new("RGBA", (144, 144), (200, 200, 200, 255))
    preview.alpha_composite(closed_hand.resize((128, 128), Image.NEAREST), (8, 8))
    preview.save(Path(sys.argv[2]) if len(sys.argv) > 2 else out / "preview.png")


if __name__ == "__main__":
    main()
