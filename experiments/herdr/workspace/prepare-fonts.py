"""Generate private static font instances from the user's installed Anthropic fonts.

No font binaries are downloaded, committed, or installed into macOS. Herdr loads
the generated files from its own profile. Static instances avoid GPUI selecting
the wrong named weight from these variable files.
"""
import argparse
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for role in ("Sans", "Serif"):
        for style, weight, italic in (
            ("Regular", 400, False),
            ("Semibold", 600, False),
            ("Italic", 400, True),
            ("Semibold Italic", 600, True),
        ):
            original = args.source / f"Anthropic{role}-{'Italic' if italic else 'Roman'}-Variable.ttf"
            font = TTFont(original)
            axes = {"wght": weight, "opsz": 16}
            axes = {key: value for key, value in axes.items() if any(a.axisTag == key for a in font["fvar"].axes)}
            font = instantiateVariableFont(font, axes, inplace=True)
            family = f"Shina {role}"
            postscript = f"Shina{role}-{style.replace(' ', '')}"
            # Keep original copyright/license records. Replace only family/style IDs.
            names = {1: family, 2: style, 3: postscript, 4: f"{family} {style}", 6: postscript, 16: family, 17: style}
            for record in list(font["name"].names):
                if record.nameID in names:
                    font["name"].setName(names[record.nameID], record.nameID, record.platformID, record.platEncID, record.langID)
            font["OS/2"].usWeightClass = weight
            font["OS/2"].fsSelection &= ~((1 << 0) | (1 << 5) | (1 << 6))
            font["OS/2"].fsSelection |= (1 << 0) if italic else 0
            font["OS/2"].fsSelection |= (1 << 5) if weight >= 600 else (0 if italic else 1 << 6)
            font["head"].macStyle = (1 if weight >= 600 else 0) | (2 if italic else 0)
            font.save(args.output / f"{postscript}.ttf")
    print("Prepared private Shina Sans and Serif instances from installed fonts.")


if __name__ == "__main__":
    main()
