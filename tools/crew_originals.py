"""Generate Shina's original crew portraits.

Usage: py -I tools/crew_originals.py [output-dir]   (default: assets/crew)
       py -I tools/crew_originals.py --sheet out.png   (a review sheet of every captain)

Twelve original captains drawn as 40x40 busts in the style of Pokemon Mystery Dungeon
portraits: a close crop, outlines tinted by the region they border, and three-tone
shading. Each sits on the square colored tile Capy uses for its crew. Six expressions
are exported per captain at 6x nearest-neighbor scale so Rune's linear filtering stays
crisp at sidebar sizes, plus four animation strips (`<Clip>.strip.png`, see CLIPS).

  ember    Platform  long layered copper hair, goggles on the forehead, olive work jacket
  juniper  Release   long mint hime-cut hair, star clip, cream cardigan with navy collar
  sable    Support   long straight indigo hair, coral headphones, grey hoodie
  pip      Research  long white-blonde waves, a ribboned top bun, green scholar coat
  bram     Operations  white captain's cap, grey moustache, navy peacoat
  wren     Interface   mustard beanie with a pompom, chestnut bob, freckles, striped scarf
  rook     Security    indigo hood, dark fringe, gold eyes, silver clasp
  nia      Data        centre-parted braids with gold cuffs, yellow headband, teal turtleneck
  jett     Performance pink mohawk, shaved sides, leather jacket, cheek plaster
  otto     Review      bald, white beard and brows, round glasses, burgundy cardigan
  bolt     Automation  robot with an antenna, ear bolts and a glowing screen face
  kit      Testing     fox ears, spiky russet hair, cream hoodie, blue neckerchief
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
            "c": 3, "C": 3, "e": 3, "E": 3, "s": 1, "S": 1,
            # Headwear and accessories used only by the later captains.
            "p": 5, "P": 5, "r": 5, "R": 5, "n": 4, "u": 3, "U": 3, "y": 3, "D": 4}
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
    def __init__(self, marks=True):
        self.g = [["." for _ in range(N)] for _ in range(N)]
        # Corner marks ("...", "z") only on full busts; small tiles get a UI status badge.
        self.marks = marks

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


def eyes_half(cv):
    # Blink in-between: the upper lid has dropped two rows.
    for x0 in EYES:
        cv.mask(17, x0, "lllll", None)
        cv.mask(18, x0, "liIIl", None)
        cv.mask(19, x0, "ljjjl", None)
        cv.mask(20, x0, ".lll.", None)


def eyes_closed(cv):
    for x0 in EYES:
        cv.mask(18, x0, "lllll", None)


def open_or(cv, eyes):
    {"half": eyes_half, "closed": eyes_closed}.get(eyes, eyes_open)(cv)


def brows(cv, left, right):
    for x, y in left + right:
        cv.put(x, y, "l")


def expression(cv, face, eyes=None):
    if face == "Normal":
        open_or(cv, eyes)
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
        # Working: focused, not angry. Level brows, open eyes, a "..." in the tile corner.
        open_or(cv, eyes)
        brows(cv, [(12, 13), (13, 13), (14, 13), (15, 13)], [(24, 13), (25, 13), (26, 13), (27, 13)])
        cv.span(24, 19, 20, "M")
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


# Tile-corner marks. They are drawn after any body motion, so they stay put on the tile.
Z_MARK = ((32, 2), (33, 2), (34, 2), (35, 2), (34, 3), (33, 4), (32, 5), (33, 5), (34, 5), (35, 5))
BANG = ((34, 1), (34, 2), (34, 3), (34, 4), (34, 6))
SPARKLE = ((34, 2), (34, 3), (32, 4), (33, 4), (34, 4), (35, 4), (36, 4), (34, 5), (34, 6))


def corner(cv, face, mark=None):
    """The face's own mark, or an animation's: ("dots", n) lights n of the three dots."""
    if not cv.marks:
        return
    if mark is None:
        mark = {"Determined": ("dots", 3), "Sad": "z"}.get(face)
    if isinstance(mark, tuple) and mark[0] == "dots":
        for i, x in enumerate((32, 34, 36)):
            cv.put(x, 4, "w" if i < mark[1] else "v")
    elif mark == "z":
        for x, y in Z_MARK:
            cv.put(x, y, "w")
    elif mark == "bang":
        for x, y in BANG:
            cv.put(x, y, "w")
    elif mark == "sparkle":
        for x, y in SPARKLE:
            cv.put(x, y, "w")


# Rows above the shoulders. Breathing moves only these; a hop moves the whole bust.
SHOULDERS = 31


def shift(cv, dy, whole):
    """Move the character dy rows (positive is down), leaving the tile in place."""
    if dy == 0:
        return
    g = cv.g
    rows = N if whole else SHOULDERS
    old = [row[:] for row in g]
    for y in range(rows):
        src = y - dy
        if src < 0:
            g[y] = ["."] * N
        elif src >= N:
            g[y] = old[N - 1][:]
        else:
            g[y] = old[src][:]


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


def ears(cv):
    """Ears for captains whose hair or headwear leaves them showing."""
    for y in range(15, 21):
        cv.put(8, y, "s"); cv.put(31, y, "S")
    for y in range(16, 20):
        cv.put(7, y, "s"); cv.put(32, y, "S")
    cv.put(8, 17, "k"); cv.put(8, 18, "k"); cv.put(31, 17, "k"); cv.put(31, 18, "k")


def bram(cv):
    ears(cv)
    head(cv)
    bust(cv)
    # Double-breasted peacoat over a white collar, with brass buttons.
    cv.spans({31: (16, 23), 32: (17, 22), 33: (18, 21), 34: (19, 20)}, "e")
    for i in range(4):
        cv.put(15 + i, 32 + i, "C"); cv.put(24 - i, 32 + i, "C")
    for x, y in ((14, 35), (14, 38), (25, 35), (25, 38)):
        cv.put(x, y, "a")
    # Close-cropped grey hair and a grey moustache.
    for y in range(11, 19):
        cv.span(y, 9, 10, "h"); cv.span(y, 29, 30, "h")
    cv.put(10, 19, "h"); cv.put(29, 19, "h")
    cv.span(22, 16, 23, "h")
    cv.mask(23, 15, "hhh....hhh", None)
    cv.put(16, 22, "H"); cv.put(23, 22, "H")
    # Captain's cap: a white crown wider than the band, a navy band, a black peak.
    cv.spans({1: (13, 26), 2: (10, 29), 3: (8, 31), 4: (8, 31), 5: (8, 31)}, "p")
    cv.span(6, 8, 31, "P")
    for y in range(2, 6):
        cv.span(y, 28, 31 if y > 2 else 29, "P")
    cv.mask(2, 13, "xxx", None); cv.mask(3, 11, "xx", None)
    cv.spans({7: (9, 30), 8: (9, 30)}, "n")
    cv.spans({9: (9, 30), 10: (10, 29)}, "d")
    cv.mask(9, 12, "DDDDDD", None)
    cv.span(11, 11, 28, "S")
    # Brass badge on the band.
    cv.mask(4, 19, "aa", None); cv.mask(5, 18, "aAAa", None)
    cv.mask(6, 18, "aaaa", None); cv.mask(7, 19, "AA", None)


def wren(cv):
    head(cv)
    bust(cv)
    # Chestnut bob under the beanie, flicking out at the ends.
    for y in range(11, 26):
        cv.span(y, 7, 10, "h"); cv.span(y, 29, 32, "h")
        cv.put(7, y, "H"); cv.put(32, y, "H")
    cv.mask(26, 6, "HH.", None); cv.mask(26, 32, ".HH", None)
    cv.mask(25, 6, "h", None); cv.mask(25, 33, "h", None)
    cv.span(12, 9, 30, "h")
    cv.mask(13, 9, "####.#####.##.#####.##", "h")
    cv.mask(14, 9, "##....###.....##....##", "h")
    cv.mask(13, 14, "g", None); cv.mask(12, 24, "gg", None)
    # Mustard knit beanie with a ribbed cuff and a cream pompom.
    cv.spans({0: (18, 21), 1: (17, 22), 2: (17, 22)}, "p")
    cv.mask(0, 18, "x", None); cv.mask(1, 17, "x", None); cv.mask(2, 21, "PP", None)
    cv.spans({3: (14, 25), 4: (12, 27), 5: (10, 29), 6: (9, 30), 7: (9, 30), 8: (9, 30)}, "r")
    for x in range(11, 30, 3):
        for y in range(4, 9):
            if cv.get(x, y) == "r":
                cv.put(x, y, "R")
    cv.spans({9: (8, 31), 10: (8, 31), 11: (8, 31)}, "r")
    for x in range(8, 32, 2):
        for y in (9, 10, 11):
            cv.put(x, y, "R")
    # Freckles.
    for x, y in ((11, 22), (13, 22), (12, 23), (26, 22), (28, 22), (27, 23)):
        cv.put(x, y, "f")
    # Striped scarf wrapped high, one end hanging.
    cv.spans({28: (15, 24), 29: (14, 25), 30: (13, 26), 31: (13, 26), 32: (14, 25)}, "u")
    for y in (29, 31):
        cv.span(y, 14, 25, "y")
    cv.span(32, 14, 25, "U")
    for y in range(33, N):
        cv.span(y, 21, 24, "u" if y % 3 else "y")
        cv.put(24, y, "U")


def rook(cv):
    # The hood's outer shape, then the face, then the hood again around the opening.
    cv.spans({1: (18, 21), 2: (16, 23), 3: (14, 25), 4: (12, 27), 5: (11, 28), 6: (10, 29), 7: (9, 30)}, "h")
    for y in range(8, 34):
        w = 1 if y > 18 else 0
        cv.span(y, 8 - w, 31 + w, "h")
    head(cv)
    bust(cv, "h", "H")
    for y in range(8, 11):
        cv.span(y, 8, 31, "h")
    for y in range(11, 28):
        w = 1 if y > 18 else 0
        cv.span(y, 8 - w, 9, "h"); cv.span(y, 30, 31 + w, "h")
        cv.put(9, y, "H"); cv.put(30, y, "H")
    cv.span(10, 11, 28, "H")
    # The hood folds in under the chin to the clasp.
    for y in range(26, 33):
        inset = min(y - 25, 6)
        cv.span(y, 9, 9 + inset, "h"); cv.span(y, 30 - inset, 30, "h")
        cv.put(9 + inset, y, "H"); cv.put(30 - inset, y, "H")
    cv.mask(3, 15, "gg", None); cv.mask(4, 13, "gg", None); cv.mask(5, 12, "g", None)
    for y in range(6, 12):
        cv.put(11 if y < 9 else 10, y, "g")
    # A dark fringe falling from under the hood.
    cv.span(11, 11, 28, "n")
    cv.mask(12, 11, "nnn.nnnnnn..nnn.nn", None)
    cv.mask(13, 11, "n...nn.nn.....n..n", None)
    for x in range(11, 29):
        if cv.get(x, 13) == "s":
            cv.put(x, 13, "S")
        if cv.get(x, 12) == "s":
            cv.put(x, 12, "S")
    # Silver clasp.
    cv.mask(31, 18, ".aa.", None); cv.mask(32, 18, "aAAa", None); cv.mask(33, 18, ".AA.", None)


def nia(cv):
    head(cv)
    bust(cv)
    # Turtleneck.
    cv.spans({28: (16, 23), 29: (15, 24), 30: (15, 24)}, "c")
    cv.span(28, 16, 23, "C"); cv.span(30, 15, 24, "C")
    cv.mask(31, 17, "e", None); cv.mask(32, 18, "e", None); cv.mask(33, 19, "ee", None)
    cv.mask(32, 21, "e", None); cv.mask(31, 22, "e", None)
    # Hair pulled back from a centre part into two long braids.
    cv.spans({3: (15, 24), 4: (13, 26), 5: (11, 28), 6: (10, 29)}, "h")
    for y in range(7, 11):
        cv.span(y, 9, 30, "h")
    cv.mask(11, 9, "####..............####", "h")
    cv.mask(12, 9, "##..................##", "h")
    for y in range(13, 17):
        cv.put(9, y, "h"); cv.put(30, y, "h")
    for y in range(4, 11):
        cv.put(19, y, "H")
    cv.mask(5, 14, "gg", None); cv.mask(6, 12, "gg", None); cv.mask(5, 23, "gg", None)
    # Headband.
    cv.span(8, 9, 30, "q"); cv.span(9, 9, 30, "Q")
    for side in (6, 30):
        for y in range(12, 37):
            seg = (y - 12) % 3
            cv.span(y, side, side + 3, "H" if seg == 2 else "h")
            if seg == 0:
                cv.put(side + 1, y, "g")
        cv.span(33, side, side + 3, "a"); cv.span(34, side, side + 3, "A")
        cv.span(37, side + 1, side + 2, "h"); cv.span(38, side + 1, side + 2, "H")
    # Gold studs.
    cv.put(9, 20, "a"); cv.put(30, 20, "a")


MOHAWK = [
    "...#........",
    "..##...#....",
    "..###.##....",
    "..######..#.",
    "..#########.",
]


def jett(cv):
    ears(cv)
    head(cv)
    bust(cv)
    # Leather jacket open over a white tee, with a silver zip and a pin.
    cv.spans({31: (16, 23), 32: (17, 22), 33: (18, 21)}, "e")
    for i in range(4):
        cv.put(15 + i, 31 + i, "C"); cv.put(24 - i, 31 + i, "C")
    cv.mask(31, 12, "CCC", None); cv.mask(31, 25, "CCC", None)
    for y in range(34, N):
        cv.put(24, y, "a")
    cv.put(13, 35, "q"); cv.put(12, 36, "q"); cv.put(14, 36, "q"); cv.put(13, 37, "q")
    # Shaved sides.
    for y in range(8, 13):
        for x in range(N):
            if cv.get(x, y) == "s":
                cv.put(x, y, "u")
    for y in range(11, 15):
        cv.put(9, y, "u"); cv.put(30, y, "u")
    # A tall pink fin with a lock falling forward.
    for i, row in enumerate(MOHAWK):
        cv.mask(i, 14, row, "h")
    for y in range(5, 12):
        cv.span(y, 16, 23, "h")
    cv.mask(12, 18, "hhhhhh", None); cv.mask(13, 20, "hhh", None); cv.mask(14, 21, "hh", None)
    for y in range(3, 11):
        cv.put(17, y, "g"); cv.put(23, y, "H")
    cv.put(24, 4, "H"); cv.put(22, 11, "H"); cv.put(23, 12, "H")
    cv.put(17, 0, "g"); cv.put(16, 1, "g")
    # Silver earring and a plaster on the cheek.
    cv.put(7, 21, "a")
    cv.mask(22, 25, "xXx", None)


def otto(cv):
    ears(cv)
    head(cv)
    bust(cv)
    # Cardigan over a collared shirt.
    cv.spans({31: (13, 26), 32: (15, 24)}, "e")
    for y in range(33, N):
        cv.span(y, 17, 22, "E")
    for y in range(34, N, 2):
        cv.put(16, y, "a")
    # Shiny scalp and a white fringe around the back.
    cv.mask(9, 15, "xx", None); cv.mask(10, 13, "xx", None); cv.put(12, 11, "x")
    for y in range(12, 20):
        cv.span(y, 9, 10, "h"); cv.span(y, 29, 30, "h")
        cv.put(9, y, "H"); cv.put(30, y, "H")
    # Full white beard and moustache.
    beard = {20: [(9, 11), (28, 30)], 21: [(9, 12), (27, 30)], 22: [(9, 14), (25, 30)],
             23: [(10, 17), (22, 29)], 24: [(10, 17), (22, 29)], 25: [(11, 28)], 26: [(11, 28)],
             27: [(12, 27)], 28: [(13, 26)], 29: [(14, 25)], 30: [(16, 23)], 31: [(18, 21)]}
    for y, parts in beard.items():
        for a, b in parts:
            cv.span(y, a, b, "h")
    for y in range(23, 31):
        a, b = beard[y][-1]
        cv.put(b, y, "H")
    cv.mask(26, 15, "H...H...H", None); cv.mask(28, 17, "H...H", None)
    cv.span(22, 16, 23, "h")
    cv.mask(23, 15, "hhH....Hhh", None)
    glasses(cv)


def otto_brows(cv, face):
    """Bushy white brows: the face's own brows recoloured, or a resting pair."""
    if face in ("Normal", "Happy", "Dizzy", "Sad"):
        cv.mask(12, 12, "#####", "l"); cv.mask(12, 23, "#####", "l")
    for y in (11, 12, 13):
        for x in range(N):
            if cv.get(x, y) == "l":
                cv.put(x, y, "h")
                if cv.get(x, y + 1) in ("s", "S"):
                    cv.put(x, y + 1, "H")


def bolt(cv):
    # Antenna.
    for y in range(2, 6):
        cv.span(y, 19, 20, "a")
    cv.mask(0, 18, ".qq.", None); cv.mask(1, 18, "qwqq", None); cv.mask(2, 18, "QqqQ", None)
    # Rounded metal head with a dark face screen.
    cv.spans({5: (13, 26), 6: (10, 29), 7: (9, 30)}, "s")
    for y in range(8, 26):
        cv.span(y, 8, 31, "s")
    cv.spans({26: (9, 30), 27: (11, 28)}, "s")
    for y in range(8, 26):
        cv.span(y, 30, 31, "S")
    cv.span(26, 9, 30, "S"); cv.span(27, 11, 28, "S"); cv.span(7, 28, 30, "S")
    cv.mask(6, 12, "xxx", None); cv.mask(7, 10, "xx", None)
    for y in range(8, 12):
        cv.put(9, y, "x")
    cv.mask(9, 13, "S", None); cv.mask(9, 26, "S", None)
    cv.spans({11: (11, 28), 25: (11, 28)}, "n")
    for y in range(12, 25):
        cv.span(y, 10, 29, "n")
    # Ear bolts.
    for y in range(14, 20):
        cv.span(y, 5, 7, "a")
    cv.span(13, 6, 7, "a"); cv.span(20, 6, 7, "a")
    for y in range(15, 19):
        cv.put(6, y, "A")
    for y in range(14, 20):
        cv.span(y, 32, 34, "a")
    cv.span(13, 32, 33, "a"); cv.span(20, 32, 33, "a")
    for y in range(15, 19):
        cv.put(33, y, "A")
    # Ribbed neck and a chest plate with a status light.
    for y in range(28, 31):
        cv.span(y, 16, 23, "d" if y != 29 else "D")
    bust(cv)
    cv.span(31, 12, 27, "e")
    cv.mask(34, 18, "qq", None); cv.mask(35, 18, "QQ", None)
    for y in (34, 36, 38):
        cv.span(y, 10, 13, "e")


def kit(cv):
    head(cv)
    bust(cv)
    # Cream hoodie with a blue neckerchief.
    cv.spans({31: (15, 24), 32: (16, 23), 33: (17, 22), 34: (18, 21), 35: (19, 20)}, "q")
    cv.span(31, 15, 24, "Q")
    cv.put(16, 32, "Q"); cv.put(23, 32, "Q")
    # Fox ears with dark tips and cream insides.
    for y in range(0, 8):
        cv.span(y, 9, min(9 + y, 15), "h")
        cv.span(y, max(30 - y, 24), 30, "h")
        cv.put(9, y, "H"); cv.put(30, y, "H")
    for y in (0, 1):
        cv.span(y, 9, 9 + y, "d"); cv.span(y, 30 - y, 30, "d")
    for y in range(3, 7):
        cv.span(y, 10, 8 + y - 1 if y < 6 else 13, "p")
        cv.span(y, 31 - y + 1 if y < 6 else 26, 29, "p")
    # Spiky russet hair.
    cv.spans({5: (13, 26), 6: (11, 28)}, "h")
    for y in range(7, 12):
        cv.span(y, 9, 30, "h")
    cv.mask(12, 9, "##########.###########", "h")
    cv.mask(13, 9, "##.####...####..####.#", "h")
    cv.mask(14, 9, "#...##.....##....##..#", "h")
    cv.mask(15, 9, "#....#............#...", "h")
    for y in range(12, 24):
        cv.span(y, 7, 9, "h"); cv.span(y, 30, 32, "h")
        cv.put(7, y, "H"); cv.put(32, y, "H")
    # Cheek tufts tipped in cream.
    cv.mask(24, 6, "Hhh", None); cv.mask(25, 6, "yh", None); cv.mask(26, 6, "y", None)
    cv.mask(24, 31, "hhH", None); cv.mask(25, 32, "hy", None); cv.mask(26, 33, "y", None)
    cv.mask(8, 13, "gg", None); cv.mask(9, 12, "g", None); cv.mask(8, 22, "ggg", None)


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
        "h": "#ecd28a", "H": "#c2a45e", "g": "#fff4cf",
        "i": "#3f78d8", "I": "#1f3f8c", "j": "#8cb8f4",
        "c": "#2f5a44", "C": "#1f3e2f", "e": "#f4f0e6", "E": "#cfc8b8",
        "a": "#e2607a", "A": "#a83a52", "d": "#5a3e2c", "q": "#5a3e2c", "Q": "#5a3e2c",
    }),
    # Added later. Indices are saved in captain profiles, so new captains only append.
    ("bram", bram, None, "#4e9c94", {
        "s": "#a26a46", "S": "#7e4e32", "k": "#6a3a24", "b": "#c47a62",
        "h": "#d6d2cc", "H": "#a29c98", "g": "#f4f2ee",
        "i": "#8a5a2a", "I": "#4a2a14", "j": "#e0a868",
        "c": "#2c3c66", "C": "#1c274a", "e": "#f2efe6", "E": "#cfc8b8",
        "a": "#e8b844", "A": "#a87e26", "d": "#1e2230", "D": "#3c4258",
        "p": "#f4f2ec", "P": "#c4c0b8", "x": "#ffffff", "n": "#2a3658",
    }),
    ("wren", wren, None, "#9a6098", {
        "s": "#f8e2cc", "S": "#e6b8a0", "k": "#d89a84", "f": "#d08a6a",
        "h": "#7a4630", "H": "#52301f", "g": "#a8684a",
        "i": "#3f8a5a", "I": "#1f5232", "j": "#94d4a4",
        "c": "#3f8a86", "C": "#2a625e", "e": "#efe6d2", "E": "#c9bda4",
        "r": "#e8ac3c", "R": "#b07a22", "p": "#f6efe0", "P": "#cfc4ae", "x": "#ffffff",
        "u": "#d4504a", "U": "#982e2c", "y": "#f4ead6",
    }),
    ("rook", rook, None, "#8f9e5e", {
        "s": "#dca47e", "S": "#b67e5c", "k": "#a86c4c",
        "h": "#3a3a6e", "H": "#232448", "g": "#5e5e9a",
        "i": "#e0b040", "I": "#9a6a1a", "j": "#f8e08a",
        "n": "#1e1a26", "a": "#d4d8e0", "A": "#8a8e9c",
    }),
    ("nia", nia, None, "#cf6f5c", {
        "s": "#7c4a32", "S": "#5e3524", "k": "#4a2818", "b": "#b0604c",
        "h": "#2e2430", "H": "#18121a", "g": "#5e4c62",
        "i": "#c87a3a", "I": "#7a3e14", "j": "#f0b070",
        "c": "#2f8a7a", "C": "#1e5e52", "e": "#f4c040",
        "q": "#f4c040", "Q": "#c0901e", "a": "#f4c040", "A": "#b08020",
    }),
    ("jett", jett, None, "#c9a23c", {
        "s": "#f0cca8", "S": "#d4a684", "k": "#c08a6a", "u": "#c8a088",
        "h": "#ec4f8c", "H": "#a82a5e", "g": "#ff9ec4",
        "i": "#2aa0c4", "I": "#16607e", "j": "#8adcf0",
        "c": "#36323c", "C": "#1e1c22", "e": "#f2f0ec", "E": "#c8c4bc",
        "a": "#d4d8e0", "A": "#8a8e9c", "q": "#ec4f8c", "x": "#f4e2bc", "X": "#d4b88a",
    }),
    ("otto", otto, otto_brows, "#7c8a9c", {
        "s": "#f4c8b0", "S": "#dca088", "k": "#c88872", "b": "#ee9c9c",
        "h": "#f0f0f2", "H": "#b4b8c4", "g": "#ffffff", "x": "#fff2e8",
        "i": "#4a72b8", "I": "#24427e", "j": "#a4c2f0",
        "c": "#8a3442", "C": "#5e2230", "e": "#f2ece0", "E": "#d8cfbe",
        "a": "#e8c070", "A": "#a8843e", "d": "#6a4a2e",
    }),
    ("bolt", bolt, None, "#4c6cc4", {
        "s": "#d4dae4", "S": "#9ea8ba", "x": "#ffffff", "n": "#1c2436",
        "l": "#7ef4e4", "w": "#ffffff", "i": "#4fe0e0", "I": "#2aa6b8", "j": "#c0fff6",
        "m": "#7ef4e4", "M": "#4fd0d0", "b": "#ff8ab0", "t": "#7ef4e4",
        "a": "#5a6478", "A": "#3a4254", "q": "#ff6a5a", "Q": "#c84038",
        "d": "#3a4254", "D": "#262c3a", "c": "#bcc6d6", "C": "#8a96aa", "e": "#6a7488",
    }),
    ("kit", kit, None, "#4a9a6a", {
        "s": "#f8dcc0", "S": "#e2b498", "k": "#d29a80",
        "h": "#e0702e", "H": "#a04418", "g": "#ffa868", "y": "#fff0dc",
        "i": "#e8a030", "I": "#a0601a", "j": "#ffd27a",
        "c": "#efe4cc", "C": "#cbbd9c", "e": "#efe4cc", "E": "#cbbd9c",
        "q": "#3a7ac0", "Q": "#28568c", "d": "#3a2418", "p": "#fbe6d0",
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


def render(draw, extra, tile, colors, face, marks=True, eyes=None, dy=0, whole=False, mark=None):
    cv = Canvas(marks)
    draw(cv)
    expression(cv, face, eyes)
    if extra:
        extra(cv, face)
    shift(cv, dy, whole)
    corner(cv, face, mark)
    finish(cv)
    pal = {k: hexc(v) for k, v in {**COMMON, **colors}.items()}
    # An unlit working dot: white sunk most of the way into the tile.
    pal["v"] = tuple(int(w * 0.35 + t * 0.65) for w, t in zip(pal["w"], hexc(tile)))
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
SMALL_CROP = (6, 4, 34, 32)


# Animation strips: frames left to right, each a full 40x40 bust. Frame 0 is always the
# matching static face, so a paused or reduced-motion portrait shows exactly that face.
# `src/visuals.rs` (`Clip`) holds the timing; keep the frame order in step with it.
CLIPS = {
    # Ready: breathing (the head settles a pixel) and a blink.
    "Idle": [
        dict(face="Normal"),
        dict(face="Normal", dy=1),
        dict(face="Normal", eyes="half"),
        dict(face="Normal", eyes="closed"),
    ],
    # Working: the corner dots type out one by one, the head dips on the second, and an
    # occasional blink.
    "Working": [
        dict(face="Determined"),
        dict(face="Determined", mark=("dots", 1)),
        dict(face="Determined", mark=("dots", 2), dy=1),
        dict(face="Determined", eyes="closed"),
    ],
    # Needs you: a startled hop with a "!" in the corner, then the plain Surprised face.
    "Attention": [
        dict(face="Surprised"),
        dict(face="Surprised", dy=-2, whole=True, mark="bang"),
        dict(face="Surprised", dy=-1, whole=True, mark="bang"),
        dict(face="Surprised", mark="bang"),
    ],
    # Done: a small hop with a sparkle, once, then the Happy face.
    "Happy": [
        dict(face="Happy"),
        dict(face="Happy", dy=-2, whole=True, mark="sparkle"),
        dict(face="Happy", dy=-1, whole=True, mark="sparkle"),
    ],
}


def strip(draw, extra, tile, colors, frames):
    size = N * SCALE
    sheet = Image.new("RGBA", (size * len(frames), size))
    for i, frame in enumerate(frames):
        sheet.paste(render(draw, extra, tile, colors, **frame), (i * size, 0))
    return sheet


def review_sheet(path):
    """A review sheet: one row per captain with the six faces at 2x (80 px), then the
    Normal face at the sidebar's 44, 36 and 24 px (head crop) sizes, smoothly scaled the
    way Rune samples them."""
    big, gap = 80, 8
    sizes = (44, 36, 24)
    width = gap + len(FACES) * (big + gap) + sum(s + gap for s in sizes) + gap
    img = Image.new("RGBA", (width, gap + len(CREW) * (big + gap)), (32, 30, 38, 255))
    for row, (name, draw, extra, tile, colors) in enumerate(CREW):
        y = gap + row * (big + gap)
        for col, face in enumerate(FACES):
            full = render(draw, extra, tile, colors, face)
            img.paste(full.resize((big, big), Image.NEAREST), (gap + col * (big + gap), y))
        x = gap + len(FACES) * (big + gap) + gap
        for size in sizes:
            if size <= 24:
                box = tuple(v * SCALE for v in SMALL_CROP)
                face = render(draw, extra, tile, colors, "Normal", marks=False).crop(box)
            else:
                face = render(draw, extra, tile, colors, "Normal")
            img.paste(face.resize((size, size), Image.LANCZOS), (x, y + (big - size) // 2))
            x += size + gap
    img.save(path)


def main():
    if len(sys.argv) > 2 and sys.argv[1] == "--sheet":
        review_sheet(sys.argv[2])
        return
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "assets/crew")
    for name, draw, extra, tile, colors in CREW:
        (out / name).mkdir(parents=True, exist_ok=True)
        for face in FACES:
            full = render(draw, extra, tile, colors, face)
            full.save(out / name / f"{face}.png", optimize=True)
            box = tuple(v * SCALE for v in SMALL_CROP)
            plain = render(draw, extra, tile, colors, face, marks=False)
            plain.crop(box).save(out / name / f"{face}.small.png", optimize=True)
        for clip, frames in CLIPS.items():
            sheet = strip(draw, extra, tile, colors, frames)
            sheet.save(out / name / f"{clip}.strip.png", optimize=True)


if __name__ == "__main__":
    main()
