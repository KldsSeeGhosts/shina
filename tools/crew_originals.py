"""Generate Shina's original crew portraits.

Usage: py -I tools/crew_originals.py [output-dir]   (default: assets/crew)

Four original captains drawn as 40x40 busts in the style of Pokemon Mystery Dungeon
portraits: a close crop, outlines tinted by the region they border, and three-tone
shading. Each sits on the square colored tile Capy uses for its crew. Six expressions
are exported per captain at 6x nearest-neighbor scale so Rune's linear filtering stays
crisp at sidebar sizes.

  ember    Platform  long layered copper hair, goggles on the forehead, olive work jacket
  juniper  Release   long mint hime-cut hair, star clip, cream cardigan with navy collar
  sable    Support   long straight indigo hair, coral headphones, grey hoodie
  pip      Research  long white-blonde waves, a ribboned top bun, green scholar coat
"""
import sys
from pathlib import Path

from PIL import Image

N = 40
SCALE = 6
FACES = ["Normal", "Happy", "Determined", "Surprised", "Dizzy", "Sad"]
HAIR = set("hHg")
# Outline color comes from the neighbor with the highest priority.
PRIORITY = {"h": 5, "H": 5, "g": 5, "a": 4, "A": 4, "d": 4, "q": 4, "Q": 4,
            "c": 3, "C": 3, "e": 3, "E": 3, "s": 1, "S": 1}
COMMON = {
    "s": "#f8e0cc", "S": "#e4b8a2", "k": "#d39a86", "l": "#2a1b26", "w": "#ffffff",
    "b": "#f3a0a8", "m": "#b45d68", "M": "#7e3846", "t": "#9ad9f4",
}


def hexc(value):
    value = value.lstrip("#")
    return tuple(int(value[i:i + 2], 16) for i in (0, 2, 4))


def shade(rgb, amount):
    return tuple(int(c * amount) for c in rgb)


class Canvas:
    def __init__(self):
        self.g = [["." for _ in range(N)] for _ in range(N)]

    def put(self, x, y, c):
        if 0 <= x < N and 0 <= y < N and c is not None:
            self.g[y][x] = c

    def span(self, y, a, b, c):
        for x in range(a, b + 1):
            self.put(x, y, c)

    def get(self, x, y):
        return self.g[y][x] if 0 <= x < N and 0 <= y < N else "."

    def mask(self, y, x0, pattern, c):
        for i, ch in enumerate(pattern):
            if ch != ".":
                self.put(x0 + i, y, c if ch == "#" else ch)

    def spans(self, table, c):
        for y, (a, b) in table.items():
            self.span(y, a, b, c)


# Shared anatomy -------------------------------------------------------------------

HEAD = {8: (14, 25), 9: (12, 27), 10: (10, 29)}
HEAD.update({y: (9, 30) for y in range(11, 21)})
HEAD.update({21: (10, 29), 22: (10, 29), 23: (11, 28), 24: (12, 27), 25: (13, 26), 26: (15, 24), 27: (17, 22)})
BUST = {31: (12, 27), 32: (9, 30), 33: (7, 32), 34: (5, 34)}
EYES = (12, 23)


def head(cv):
    cv.spans(HEAD, "s")
    for y in range(11, 21):
        cv.put(30, y, "S")
    for x, y in ((29, 21), (29, 22), (28, 23), (27, 24), (26, 25), (24, 26), (22, 27)):
        cv.put(x, y, "S")
    cv.span(28, 17, 22, "S")
    for y in (29, 30):
        cv.span(y, 17, 22, "s")
    cv.put(20, 21, "k")


def bust(cv, c="c", C="C"):
    for y in range(31, N):
        a, b = BUST.get(y, (3, 36))
        cv.span(y, a, b, c)
        cv.span(y, max(a, b - 4), b, C)
        cv.put(a, y, C)


# Expressions ----------------------------------------------------------------------

def eyes_open(cv):
    for x0 in EYES:
        cv.mask(15, x0, "lllll", None)
        cv.mask(16, x0, "lwiil", None)
        cv.mask(17, x0, "liIIl", None)
        cv.mask(18, x0, "liIIl", None)
        cv.mask(19, x0, "ljjjl", None)
        cv.mask(20, x0, ".lll.", None)


def lidded(cv):
    for x0 in EYES:
        cv.mask(16, x0, "lllll", None)
        cv.mask(17, x0, "liIIl", None)
        cv.mask(18, x0, "liIIl", None)
        cv.mask(19, x0, "ljjjl", None)
        cv.mask(20, x0, ".lll.", None)


def brows(cv, left, right):
    for x, y in left + right:
        cv.put(x, y, "l")


def expression(cv, face):
    if face == "Normal":
        eyes_open(cv)
        cv.span(24, 19, 20, "m")
    elif face == "Happy":
        for x0 in EYES:
            cv.put(x0 + 2, 16, "l")
            cv.put(x0 + 1, 17, "l"); cv.put(x0 + 3, 17, "l")
            cv.put(x0, 18, "l"); cv.put(x0 + 4, 18, "l")
        for x in (11, 12, 13, 26, 27, 28):
            cv.put(x, 21, "b")
        cv.span(23, 18, 21, "M")
        cv.span(24, 19, 20, "m")
    elif face == "Determined":
        lidded(cv)
        brows(cv, [(11, 12), (12, 12), (13, 13), (14, 13), (15, 14), (16, 14)],
              [(23, 14), (24, 14), (25, 13), (26, 13), (27, 12), (28, 12)])
        cv.span(24, 18, 21, "M")
    elif face == "Surprised":
        for x0 in EYES:
            cv.mask(14, x0, ".lll.", None)
            cv.mask(15, x0, "lwjjl", None)
            cv.mask(16, x0, "ljjjl", None)
            cv.mask(17, x0, "ljIjl", None)
            cv.mask(18, x0, "ljjjl", None)
            cv.mask(19, x0, ".lll.", None)
        brows(cv, [(12, 12), (13, 11), (14, 11), (15, 11)], [(24, 11), (25, 11), (26, 11), (27, 12)])
        cv.mask(23, 19, "MM", None)
        cv.mask(24, 18, "MmmM", None)
        cv.mask(25, 19, "MM", None)
    elif face == "Dizzy":
        for x0 in EYES:
            cv.mask(15, x0, ".lll.", None)
            cv.mask(16, x0, "l...l", None)
            cv.mask(17, x0, "l.l.l", None)
            cv.mask(18, x0, "l..ll", None)
            cv.mask(19, x0, ".lll.", None)
        for x, y in ((18, 24), (19, 23), (20, 24), (21, 23)):
            cv.put(x, y, "M")
        for x, y in ((31, 11), (31, 12), (30, 12)):
            cv.put(x, y, "t")
    elif face == "Sad":
        # Offline: asleep rather than upset, with a small "z" in the tile corner.
        for x0 in EYES:
            cv.mask(18, x0, "lllll", None)
            cv.mask(19, x0, ".l.l.", None)
        cv.span(24, 19, 20, "m")
        for x, y in ((32, 2), (33, 2), (34, 2), (35, 2), (34, 3), (33, 4), (32, 5), (33, 5), (34, 5), (35, 5)):
            cv.put(x, y, "w")


# Captains -------------------------------------------------------------------------

def ember(cv):
    for y in range(6, N):
        cv.span(y, 8, 31, "H")
    head(cv)
    bust(cv)
    cv.mask(31, 15, "eeeeeeeeee", None)
    cv.mask(32, 16, "eeeeeeee", None)
    for y in range(32, N):
        cv.put(20, y, "a")
    for y in range(33, N):
        cv.put(9, y, "C"); cv.put(30, y, "C")
    cv.mask(35, 10, "AA", None); cv.mask(35, 28, "AA", None)
    # Messy crop with a cowlick.
    cv.spans({2: (19, 20), 3: (18, 20), 4: (14, 25), 5: (12, 27), 6: (10, 29)}, "h")
    for y in range(7, 11):
        cv.span(y, 9, 30, "h")
    cv.mask(11, 9, "######.#######.#######", "h")
    cv.mask(12, 9, "#####.#####.######.###", "h")
    cv.mask(13, 9, "###...####...####...##", "h")
    cv.mask(14, 9, "##.....##.....##.....#", "h")
    # Long layers over the shoulders, ending in uneven points.
    ends = {7: 36, 8: 38, 9: 34, 10: 31, 29: 31, 30: 34, 31: 38, 32: 36}
    for x, bottom in ends.items():
        for y in range(12, bottom + 1):
            cv.put(x, y, "H" if x in (7, 32) else "h")
    cv.mask(4, 15, "gg", None); cv.mask(5, 13, "ggg", None); cv.mask(6, 12, "g", None)
    cv.mask(12, 13, "H", None); cv.mask(12, 25, "H", None); cv.mask(11, 20, "H", None)
    # Goggles pushed up on the forehead.
    cv.span(8, 9, 30, "a")
    for x0 in (12, 22):
        cv.mask(6, x0, ".dddd.", None)
        cv.mask(7, x0, "dqQqqd", None)
        cv.mask(8, x0, "dqqqqd", None)
        cv.mask(9, x0, ".dddd.", None)


def juniper(cv):
    for y in range(6, N):
        cv.span(y, 8, 31, "H")
    head(cv)
    bust(cv, "e", "E")
    # Navy sailor-style collar.
    for y, (a, b) in {31: (12, 27), 32: (11, 28), 33: (12, 27), 34: (14, 25), 35: (16, 23), 36: (18, 21)}.items():
        cv.span(y, a, b, "c")
    cv.span(31, 16, 23, "s"); cv.span(32, 18, 21, "s")
    cv.mask(33, 13, "C", None); cv.mask(33, 26, "C", None)
    cv.mask(37, 19, "aa", None); cv.mask(38, 19, "AA", None)
    # Long straight hair with a blunt fringe and side locks.
    cv.spans({4: (14, 25), 5: (11, 28), 6: (9, 30)}, "h")
    for y in range(7, 14):
        cv.span(y, 9, 30, "h")
    cv.mask(14, 9, "#########....#########", "h")
    for y in range(11, 33):
        cv.span(y, 7, 10, "h"); cv.span(y, 29, 32, "h")
        cv.put(7, y, "H"); cv.put(32, y, "H")
    cv.mask(4, 16, "gggg", None); cv.mask(5, 13, "ggg", None); cv.mask(6, 11, "gg", None)
    cv.mask(9, 12, "g", None); cv.mask(13, 13, "H", None); cv.mask(13, 26, "H", None)
    # Star hair clip.
    cv.put(27, 9, "a"); cv.mask(10, 26, "aaa", None); cv.put(27, 11, "a"); cv.put(26, 12, "A"); cv.put(28, 12, "A")


def sable(cv):
    for y in range(6, N):
        cv.span(y, 8, 31, "H")
    head(cv)
    bust(cv)
    # Hood folds and drawstrings.
    cv.spans({31: (10, 15), 32: (8, 14)}, "C")
    cv.spans({31: (24, 29), 32: (25, 31)}, "C")
    for y in range(32, 38):
        cv.put(17, y, "e"); cv.put(22, y, "e")
    cv.put(17, 38, "E"); cv.put(22, 38, "E")
    cv.span(37, 12, 27, "C")
    # Bob with a side-swept fringe.
    cv.spans({4: (13, 26), 5: (11, 28), 6: (9, 30)}, "h")
    for y in range(7, 12):
        cv.span(y, 9, 30, "h")
    cv.mask(12, 9, "#..#####################", "h")
    cv.mask(13, 9, "#....###########...####", "h")
    cv.mask(14, 9, "#......#######.......##", "h")
    for y in range(12, 37):
        cv.span(y, 6, 9, "h"); cv.span(y, 30, 33, "h")
        cv.put(6, y, "H"); cv.put(33, y, "H")
    cv.mask(37, 7, "hh", None); cv.mask(37, 31, "hh", None)
    cv.mask(5, 14, "ggg", None); cv.mask(6, 12, "gg", None); cv.mask(13, 17, "H", None)
    # Headphones: band over the crown, cups over the ears.
    cv.span(4, 13, 26, "a"); cv.put(12, 5, "a"); cv.put(27, 5, "a")
    for y in range(6, 14):
        cv.put(10 if y < 10 else 8, y, "a"); cv.put(29 if y < 10 else 31, y, "a")
    for y in range(14, 21):
        cv.span(y, 5, 8, "a"); cv.span(y, 31, 34, "a")
        cv.put(8, y, "d"); cv.put(31, y, "d")
    cv.span(20, 5, 8, "A"); cv.span(20, 31, 34, "A")


def pip(cv):
    for y in range(5, N):
        cv.span(y, 8, 31, "H")
    head(cv)
    bust(cv)
    cv.mask(31, 15, "eeeeeeeeee", None)
    cv.mask(32, 13, "eee......eee", None)
    for y in (34, 37):
        cv.put(19, y, "a"); cv.put(20, y, "A")
    cv.span(32, 18, 21, "C")
    # Curls with a top bun.
    cv.spans({0: (17, 22), 1: (16, 23), 2: (16, 23), 3: (17, 22)}, "h")
    cv.mask(1, 17, "gg", None)
    cv.spans({4: (12, 27), 5: (10, 29), 6: (9, 30)}, "h")
    for y in range(7, 12):
        cv.span(y, 8, 31, "h")
    cv.mask(12, 8, "#####.######.######.####", "h")
    cv.mask(13, 8, "###..#..####..##..#..###", "h")
    for y in range(13, 38):
        wave = 1 if (y // 3) % 2 else 0
        cv.span(y, 6 - wave, 9, "h"); cv.span(y, 30, 33 + wave, "h")
    cv.mask(38, 6, "hh", None); cv.mask(38, 32, "hh", None)
    cv.mask(5, 13, "ggg", None); cv.mask(6, 11, "gg", None); cv.mask(9, 22, "gg", None)
    # Ribbon tied around the bun.
    cv.mask(2, 13, "aa", None); cv.mask(3, 12, "aaaA", None); cv.mask(4, 13, "aA", None)
    cv.mask(2, 25, "aa", None); cv.mask(3, 24, "Aaaa", None); cv.mask(4, 25, "Aa", None)
    cv.span(3, 16, 23, "a")


def glasses(cv):
    for x0 in (11, 22):
        cv.mask(14, x0, ".ddddd.", None)
        for y in range(15, 21):
            cv.put(x0, y, "d"); cv.put(x0 + 6, y, "d")
        cv.mask(21, x0, ".ddddd.", None)
    cv.span(17, 18, 21, "d")


CREW = [
    ("ember", ember, None, "#6a92b8", {
        "h": "#c8643a", "H": "#8e3f26", "g": "#f0a06a",
        "i": "#3c9a5a", "I": "#1f5e36", "j": "#8fd4a0",
        "c": "#6d7442", "C": "#4a5030", "e": "#efe6d2", "E": "#c9bda4",
        "a": "#3a2e2a", "A": "#c9a24a", "d": "#2f2a28", "q": "#4fb4b0", "Q": "#bff0ea",
    }),
    ("juniper", juniper, None, "#a8687a", {
        "h": "#7fcbb0", "H": "#4f9482", "g": "#c6f0df",
        "i": "#d49a2a", "I": "#8a5a12", "j": "#f3d27a",
        "c": "#2f3a66", "C": "#1f2748", "e": "#f2ead8", "E": "#cfc3a8",
        "a": "#f4c94e", "A": "#b8902c", "d": "#2f3a66", "q": "#2f3a66", "Q": "#2f3a66",
    }),
    ("sable", sable, None, "#b88a52", {
        "h": "#3a3570", "H": "#24204a", "g": "#6c64b0",
        "i": "#8a5cd0", "I": "#4e2f8a", "j": "#bfa0f0",
        "c": "#8a8f9c", "C": "#666a76", "e": "#f2f2f4", "E": "#c4c6cc",
        "a": "#ef7a62", "A": "#b0503e", "d": "#3a2a30", "q": "#ef7a62", "Q": "#ef7a62",
    }),
    ("pip", pip, None, "#8a76b0", {
        "h": "#f2e2b0", "H": "#c9b27a", "g": "#fffae6",
        "i": "#3f78d8", "I": "#1f3f8c", "j": "#8cb8f4",
        "c": "#2f5a44", "C": "#1f3e2f", "e": "#f4f0e6", "E": "#cfc8b8",
        "a": "#e2607a", "A": "#a83a52", "d": "#5a3e2c", "q": "#5a3e2c", "Q": "#5a3e2c",
    }),
]


def finish(cv):
    g = cv.g
    for y in range(1, N):
        for x in range(N):
            if g[y][x] == "s" and g[y - 1][x] in HAIR:
                g[y][x] = "S"
    edge = []
    for y in range(N):
        for x in range(N):
            if g[y][x] != ".":
                continue
            near = [cv.get(x + dx, y + dy) for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))]
            near = [c for c in near if c not in (".", "o")]
            if near:
                edge.append((x, y, max(near, key=lambda c: PRIORITY.get(c, 2))))
    for x, y, source in edge:
        g[y][x] = "o" + source


def render(draw, extra, tile, colors, face):
    cv = Canvas()
    draw(cv)
    expression(cv, face)
    if extra:
        extra(cv)
    finish(cv)
    pal = {k: hexc(v) for k, v in {**COMMON, **colors}.items()}
    img = Image.new("RGBA", (N, N), hexc(tile) + (255,))
    for y in range(N):
        for x in range(N):
            ch = cv.g[y][x]
            if ch == ".":
                continue
            if ch.startswith("o"):
                base = pal[ch[1]]
                rgb = tuple(int(c * 0.35 + d * 0.65) for c, d in zip(shade(base, 0.45), (26, 20, 32)))
            else:
                rgb = pal[ch]
            img.putpixel((x, y), rgb + (255,))
    return img.resize((N * SCALE, N * SCALE), Image.NEAREST)


# The face occupies roughly x 6..33 and y 3..30; this square crop fills small tiles with it.
SMALL_CROP = (5, 3, 35, 33)


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "assets/crew")
    for name, draw, extra, tile, colors in CREW:
        (out / name).mkdir(parents=True, exist_ok=True)
        for face in FACES:
            full = render(draw, extra, tile, colors, face)
            full.save(out / name / f"{face}.png", optimize=True)
            box = tuple(v * SCALE for v in SMALL_CROP)
            full.crop(box).save(out / name / f"{face}.small.png", optimize=True)


if __name__ == "__main__":
    main()
