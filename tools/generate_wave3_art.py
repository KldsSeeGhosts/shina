#!/usr/bin/env python3
"""Generate Shina Wave 3 anime portraits (12 Captains + 8 Provider Crewmates)
using OpenAI's gpt-image-2.6-sunburst model, then downsample and crop to the
exact sizes Shina expects (512x512 full bust, 256x256 tight head crop).

Usage:
  py tools/generate_wave3_art.py --key-file <path> --stage anchor
  py tools/generate_wave3_art.py --key-file <path> --stage captains
  py tools/generate_wave3_art.py --key-file <path> --stage mascots
  py tools/generate_wave3_art.py --key-file <path> --stage expressions
  py tools/generate_wave3_art.py --stage install   # writes 512/256 PNGs + sheets
"""

from __future__ import annotations

import argparse
import base64
import concurrent.futures
import io
import json
import mimetypes
import os
import shutil
import sys
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
MASTERS_DIR = ROOT / "artifacts" / "ai-masters"
CREW_DIR = ROOT / "assets" / "crew"
MASCOT_DIR = ROOT / "assets" / "providers" / "crew"

DEFAULT_MODEL = "gpt-image-2.6-sunburst"
DEFAULT_QUALITY = "high"

STYLE_BASE = (
    "Bust portrait of an original adult character in clean modern Japanese anime illustration style "
    "(contemporary studio cel-shaded anime key visual aesthetic). "
    "Crisp cel shading with two clean shadow tones per material and one luminous colored rim light, "
    "delicate tapered line art tinted with the local material color (never harsh thick black outlines), "
    "expressive anime eyes with a clean specular highlight and subtle iris depth, "
    "readable silhouette and hair shapes that remain clear even when scaled down to 36 pixels. "
    "Strict composition: three-quarter view with head turned about 15 degrees to the viewer's right, "
    "head and upper shoulders only, top of the hair resting around 6 to 10 percent below the top edge, "
    "eyes centered horizontally and sitting near 44 percent of the image height, chin near 68 percent, "
    "shoulders extending cleanly off the bottom edge. "
    "Soft warm key light from the upper left, crisp colored rim light along the right contour. "
    "Full-bleed square 1:1 composition with NO border, NO frame, NO rounded corners, NO text, NO watermark, "
    "NO signature, NO UI elements, NO hands, NO props held in hand, NO scenery, NO vignette, NO chibi proportions, "
    "NO 3D render look, NO photorealism, NO painterly canvas noise."
)

STYLE_REF_PREFIX = (
    "Match ONLY the anime illustration style, cel-shading rendering, colored tapered line weight, "
    "eye style, lighting quality, and exact head-and-shoulders framing/scale (eyes at ~44% height, "
    "chin at ~68% height, facing slightly right) from the reference image. "
    "Do NOT copy the reference character's identity, hair, clothes, colors, or accessories. "
    "Draw a completely distinct new character as specified below:\n\n"
)


@dataclass(frozen=True)
class CharacterSpec:
    key: str
    name: str
    role: str
    signal: str
    backdrop: str
    description: str
    expression: str


CAPTAINS: list[CharacterSpec] = [
    CharacterSpec(
        key="ember",
        name="Ember",
        role="Platform",
        signal="#e0793a",
        backdrop="#2a1c14",
        description=(
            "A sharp, energetic young adult with messy tousled copper-orange hair and bright amber eyes, "
            "wearing an olive-green utility work jacket over a heather-grey crew tee, with vintage brass "
            "aviator goggles pushed up resting on the forehead."
        ),
        expression="A lopsided, confident grin with lively eyes.",
    ),
    CharacterSpec(
        key="juniper",
        name="Juniper",
        role="Release",
        signal="#45c49b",
        backdrop="#12261f",
        description=(
            "A poised young woman with long straight pastel-mint green hair, a clean blunt fringe, and calm "
            "teal eyes, wearing a soft cream knit cardigan over a dark collar, with a small gold five-point "
            "star hair clip pinned above her left temple."
        ),
        expression="Calm and composed, a small gentle closed-mouth smile.",
    ),
    CharacterSpec(
        key="sable",
        name="Sable",
        role="Support",
        signal="#8b84f0",
        backdrop="#1a1830",
        description=(
            "A woman in her late twenties with a sleek indigo chin-length bob haircut and luminous violet eyes, "
            "wearing a clean slate-grey zip hoodie and stylish coral-red over-ear studio headphones worn on her head."
        ),
        expression="Attentive, head tilted slightly, a warm reassuring half smile.",
    ),
    CharacterSpec(
        key="pip",
        name="Pip",
        role="Research",
        signal="#d9b23a",
        backdrop="#28220f",
        description=(
            "A bright young adult scholar with fluffy white-blonde curly hair gathered in a high top bun "
            "with loose tendrils framing the face, warm hazel eyes behind thin round gold wire glasses, "
            "wearing a structured deep forest-green wool coat over a cream collar."
        ),
        expression="Curious and eager, eyebrows slightly raised, lips lightly parted.",
    ),
    CharacterSpec(
        key="bram",
        name="Bram",
        role="Operations",
        signal="#5b8fd6",
        backdrop="#131c2b",
        description=(
            "A seasoned older adult man in his fifties with neat grey hair tucked under a crisp white "
            "maritime captain's cap with a dark visor, a well-groomed full grey moustache, weathered kind features, "
            "and sharp steel-blue eyes, wearing a heavy navy-blue double-breasted peacoat with brass buttons."
        ),
        expression="Steady and dryly amused, one eyebrow slightly raised, calm half-smile.",
    ),
    CharacterSpec(
        key="wren",
        name="Wren",
        role="Interface",
        signal="#b97ad0",
        backdrop="#23162a",
        description=(
            "A cheerful young woman with a warm chestnut-brown bob, light freckles across her nose and cheeks, "
            "and lively warm brown eyes, wearing a cozy mustard-yellow knit beanie topped with a small pompom "
            "and a chunky plum-and-cream striped scarf wrapped neatly around her neck."
        ),
        expression="Bright, open, and welcoming with a genuine smile.",
    ),
    CharacterSpec(
        key="rook",
        name="Rook",
        role="Security",
        signal="#93a84c",
        backdrop="#1a1f12",
        description=(
            "A cool, observant adult with dark charcoal-black bangs peeking from beneath a raised deep-indigo "
            "hooded cloak-jacket fastened at the collar with a crisp matte silver clasp, and striking sharp gold eyes."
        ),
        expression="Watchful and calm, level gaze, mouth closed.",
    ),
    CharacterSpec(
        key="nia",
        name="Nia",
        role="Data",
        signal="#f0685c",
        backdrop="#2b1614",
        description=(
            "A confident Black woman in her late twenties with deep brown skin, long centre-parted dark braids "
            "accented with small gleaming gold braid cuffs, and dark brown eyes, wearing a sleek teal knit "
            "turtleneck and a bright golden-yellow fabric headband."
        ),
        expression="Composed and self-assured, a subtle poised smile.",
    ),
    CharacterSpec(
        key="jett",
        name="Jett",
        role="Performance",
        signal="#ee5fa0",
        backdrop="#2b1420",
        description=(
            "An edgy, dynamic young adult with a styled vibrant hot-pink mohawk and closely shaved dark sides, "
            "sharp cool grey eyes, a small neat beige adhesive plaster strip on the right cheek, wearing a sleek "
            "black leather moto jacket with a popped collar."
        ),
        expression="A sharp, fearless smirk with chin slightly up.",
    ),
    CharacterSpec(
        key="otto",
        name="Otto",
        role="Review",
        signal="#a3acba",
        backdrop="#1b1e24",
        description=(
            "A gentle older mentor in his late fifties, bald with a tidy full white beard, bushy white eyebrows, "
            "and kind pale-blue eyes behind classic round tortoise-shell/wire glasses, wearing a warm burgundy "
            "knit cardigan layered over a crisp collared shirt."
        ),
        expression="Kind, patient, and warm with softly crinkled smiling eyes.",
    ),
    CharacterSpec(
        key="bolt",
        name="Bolt",
        role="Automation",
        signal="#38bdf8",
        backdrop="#0f2029",
        description=(
            "A sleek, charming retro-futuristic anime android robot (no hair) with a smooth rounded matte cobalt-blue "
            "metal head, a single small glowing antenna on top, neat metallic ear bolts on each side, and a glossy "
            "dark visor-screen face displaying two bright glowing cyan expressive eyes, wearing a brushed steel "
            "neck collar with a cobalt chest plate. Drawn in the exact same crisp cel-shaded anime style."
        ),
        expression="Cheerful, with the two glowing cyan screen eyes curved into happy upturned arcs.",
    ),
    CharacterSpec(
        key="kit",
        name="Kit",
        role="Testing",
        signal="#5fbf62",
        backdrop="#14241a",
        description=(
            "A lively young adult kemonomimi character with spiky russet-red anime hair, fluffy fox ears with "
            "soft cream tips poking naturally from the hair, and bright emerald-green eyes, wearing a relaxed "
            "cream hoodie with a crisp cobalt-blue neckerchief tied loosely at the collar."
        ),
        expression="Alert, clever, and playful, one fox ear slightly tilted, bright mischievous smile.",
    ),
]


MASCOTS: list[CharacterSpec] = [
    CharacterSpec(
        key="openai",
        name="OpenAI Crewmate",
        role="GPT / Codex",
        signal="#10a37f",
        backdrop="#0d1e19",
        description=(
            "A cute modern anime AI crewmate character inspired by OpenAI/GPT: clean snowy-white bob hair "
            "with soft mint-emerald tips, bright emerald-green eyes, wearing a sleek charcoal-black tech hoodie "
            "with emerald trim and a small subtle glowing emerald geometric hexagonal knot emblem pin on the collar."
        ),
        expression="Bright, confident, and helpful smile.",
    ),
    CharacterSpec(
        key="anthropic",
        name="Anthropic Crewmate",
        role="Claude",
        signal="#d97757",
        backdrop="#231710",
        description=(
            "A cute modern anime AI crewmate character inspired by Claude/Anthropic: warm soft terracotta-copper "
            "wavy hair, warm golden-amber eyes, wearing a cozy cream-beige sweater and a neat burnt-terracotta "
            "knit scarf, accented with a small golden-coral geometric starburst hairclip."
        ),
        expression="Warm, thoughtful, and approachable gentle smile.",
    ),
    CharacterSpec(
        key="google",
        name="Google Crewmate",
        role="Gemini",
        signal="#5b8def",
        backdrop="#12172b",
        description=(
            "A cute modern anime AI crewmate character inspired by Gemini/Google: silky periwinkle sky-blue hair "
            "softly fading to lavender-violet at the tips, sparkling sapphire-blue eyes, a sleek silver four-point "
            "diamond star hairclip above the temple, wearing a crisp white-and-cobalt futuristic jacket collar."
        ),
        expression="Bright, curious, and starry-eyed friendly smile.",
    ),
    CharacterSpec(
        key="deepseek",
        name="DeepSeek Crewmate",
        role="DeepSeek",
        signal="#4d88ff",
        backdrop="#0e1928",
        description=(
            "A cute modern anime AI crewmate character inspired by DeepSeek: deep ocean-blue hair with a playful "
            "swept tuft, crisp bright cyan-blue eyes, wearing a cozy navy-blue hooded tech parka with white "
            "piping and a cute tiny cerulean blue whale enamel pin on the collar."
        ),
        expression="Cool, calm, and quietly focused slight smile.",
    ),
    CharacterSpec(
        key="glm",
        name="GLM Crewmate",
        role="GLM / Z.ai",
        signal="#2ec4d6",
        backdrop="#0f1f24",
        description=(
            "A cute modern anime AI crewmate character inspired by GLM/Zhipu: sleek icy silver-cyan short layered "
            "hair, luminous teal-cyan eyes, wearing a crisp minimal white-and-slate high-collar tech jacket with "
            "a tiny glowing cyan crystal prism pin at the lapel."
        ),
        expression="Sharp, serene, and analytical gentle smile.",
    ),
    CharacterSpec(
        key="qwen",
        name="Qwen Crewmate",
        role="Qwen",
        signal="#9b6bff",
        backdrop="#1b142a",
        description=(
            "A cute modern anime AI crewmate character inspired by Qwen: soft pastel-violet and lavender hair "
            "styled in a lively layered cut, bright amethyst-purple eyes, wearing a modern dark charcoal hoodie "
            "with purple accents and a tiny glowing violet diamond spark pin on the collar."
        ),
        expression="Energetic, friendly, and upbeat smile.",
    ),
    CharacterSpec(
        key="grok",
        name="Grok Crewmate",
        role="Grok / xAI",
        signal="#c8cdd6",
        backdrop="#15161a",
        description=(
            "A cute modern anime AI crewmate character inspired by Grok/xAI: sharp asymmetrical charcoal-black "
            "hair with a sleek silver-white streak, striking luminous pale-silver eyes, wearing a minimalist "
            "matte-black leather tech jacket with a clean metallic orbital ring lapel pin."
        ),
        expression="Witty, sharp, and confident sly smirk.",
    ),
    CharacterSpec(
        key="default",
        name="Default Crewmate",
        role="Local / Custom",
        signal="#8aa4ff",
        backdrop="#1a1c22",
        description=(
            "A cute modern anime AI crewmate character for a custom/local model: soft cool slate-silver medium hair, "
            "calm periwinkle-grey eyes, wearing a clean dark graphite knit pullover with a subtle glowing "
            "sky-blue spark badge on the collar."
        ),
        expression="Friendly, steady, and dependable soft smile.",
    ),
]


EXPRESSION_EDITS: dict[str, str] = {
    "Happy": (
        "Edit ONLY the facial expression of this exact character to be Happy: "
        "bright smiling eyes (curved softly or sparkling happily) and an open joyful smile. "
        "Keep the exact same character identity, hair, clothing, colors, background, rim light, "
        "head position, camera framing, and anime cel-shading style unchanged."
    ),
    "Determined": (
        "Edit ONLY the facial expression of this exact character to be Determined and focused on work: "
        "eyebrows lowered slightly in concentration, eyes narrowed slightly and intensely focused, mouth set firmly. "
        "Keep the exact same character identity, hair, clothing, colors, background, rim light, "
        "head position, camera framing, and anime cel-shading style unchanged."
    ),
    "Surprised": (
        "Edit ONLY the facial expression of this exact character to be Surprised and alert (needing user attention): "
        "eyebrows raised high, eyes wide open, mouth forming a small surprised open oval. "
        "Keep the exact same character identity, hair, clothing, colors, background, rim light, "
        "head position, camera framing, and anime cel-shading style unchanged."
    ),
    "Dizzy": (
        "Edit ONLY the facial expression of this exact character to look tired, stalled, or troubled: "
        "eyebrows tilted slightly upward in the center with worried tension, eyes half-lidded and weary, mouth small and flat. "
        "Keep the exact same character identity, hair, clothing, colors, background, rim light, "
        "head position, camera framing, and anime cel-shading style unchanged."
    ),
}

BOLT_EXPRESSION_EDITS: dict[str, str] = {
    "Happy": (
        "Edit ONLY the glowing cyan screen face of this exact robot character to look Happy: "
        "bright cheerful upturned crescent arc eyes with tiny sparkles on the dark screen. "
        "Keep the exact same metal head, antenna, collar, background, lighting, framing, and anime style unchanged."
    ),
    "Determined": (
        "Edit ONLY the glowing cyan screen face of this exact robot character to look Determined and busy: "
        "focused angled glowing cyan visor eyes tilted inward in concentration. "
        "Keep the exact same metal head, antenna, collar, background, lighting, framing, and anime style unchanged."
    ),
    "Surprised": (
        "Edit ONLY the glowing cyan screen face of this exact robot character to look Surprised: "
        "wide glowing cyan circular O-shaped eyes on the dark screen. "
        "Keep the exact same metal head, antenna, collar, background, lighting, framing, and anime style unchanged."
    ),
    "Dizzy": (
        "Edit ONLY the glowing cyan screen face of this exact robot character to look stalled/error: "
        "dimmed glowing cyan X or spiral dizzy eyes on the dark screen. "
        "Keep the exact same metal head, antenna, collar, background, lighting, framing, and anime style unchanged."
    ),
}


def build_prompt(spec: CharacterSpec, with_ref: bool = False) -> str:
    body = (
        f"{STYLE_BASE}\n\n"
        f"Rim light color: {spec.signal}. "
        f"Background: flat solid {spec.backdrop} with one subtle soft radial glow of {spec.signal} centered behind the head. "
        f"Character: {spec.description} "
        f"Expression: {spec.expression}"
    )
    if spec.key == "bolt":
        body = body.replace(
            "expressive anime eyes with a clean specular highlight and subtle iris depth",
            "expressive glowing cyan digital eyes displayed on a dark visor screen face",
        )
    return (STYLE_REF_PREFIX + body) if with_ref else body


def read_api_key(key_file: str | None) -> str:
    if key_file:
        return Path(key_file).read_text(encoding="utf-8").strip()
    key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not key:
        raise RuntimeError("No API key provided via --key-file or OPENAI_API_KEY")
    return key


def decode_image_response(payload: dict) -> bytes:
    item = payload["data"][0]
    if "b64_json" in item and item["b64_json"]:
        return base64.b64decode(item["b64_json"])
    if "url" in item and item["url"]:
        with urllib.request.urlopen(item["url"], timeout=60) as resp:
            return resp.read()
    raise RuntimeError(f"Unexpected image payload keys: {list(item.keys())}")


def api_generate(api_key: str, prompt: str, model: str, quality: str) -> bytes:
    url = "https://api.openai.com/v1/images/generations"
    body = json.dumps(
        {
            "model": model,
            "prompt": prompt,
            "size": "1024x1024",
            "quality": quality,
            "output_format": "png",
            "n": 1,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=body,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    return _send_with_retries(req)


def _encode_multipart(fields: list[tuple[str, str]], files: list[tuple[str, str, bytes, str]]) -> tuple[bytes, str]:
    boundary = f"----ShinaArtBoundary{uuid.uuid4().hex}"
    chunks: list[bytes] = []
    for name, value in fields:
        chunks.append(f"--{boundary}\r\n".encode())
        chunks.append(f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode())
        chunks.append(value.encode("utf-8"))
        chunks.append(b"\r\n")
    for name, filename, content, mime in files:
        chunks.append(f"--{boundary}\r\n".encode())
        chunks.append(
            f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\n'.encode()
        )
        chunks.append(f"Content-Type: {mime}\r\n\r\n".encode())
        chunks.append(content)
        chunks.append(b"\r\n")
    chunks.append(f"--{boundary}--\r\n".encode())
    return b"".join(chunks), f"multipart/form-data; boundary={boundary}"


def api_edit(
    api_key: str,
    prompt: str,
    ref_path: Path,
    model: str,
    quality: str,
) -> bytes:
    url = "https://api.openai.com/v1/images/edits"
    fields = [
        ("model", model),
        ("prompt", prompt),
        ("size", "1024x1024"),
        ("quality", quality),
        ("output_format", "png"),
        ("n", "1"),
    ]
    mime = mimetypes.guess_type(str(ref_path))[0] or "image/png"
    body, content_type = _encode_multipart(
        fields,
        [("image[]", ref_path.name, ref_path.read_bytes(), mime)],
    )
    req = urllib.request.Request(
        url,
        data=body,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": content_type,
        },
        method="POST",
    )
    return _send_with_retries(req)


def _send_with_retries(req: urllib.request.Request, attempts: int = 4) -> bytes:
    last_err: Exception | None = None
    for attempt in range(attempts):
        try:
            with urllib.request.urlopen(req, timeout=180) as resp:
                data = json.loads(resp.read().decode("utf-8"))
                return decode_image_response(data)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            if exc.code in (429, 500, 502, 503, 504) and attempt + 1 < attempts:
                wait = 2.5 * (attempt + 1)
                print(f"  HTTP {exc.code}, retrying in {wait:.1f}s...", flush=True)
                time.sleep(wait)
                last_err = RuntimeError(f"HTTP {exc.code}: {body}")
                continue
            raise RuntimeError(f"OpenAI API HTTP {exc.code}: {body}") from exc
        except Exception as exc:
            if attempt + 1 < attempts:
                wait = 2.0 * (attempt + 1)
                print(f"  Error ({exc}), retrying in {wait:.1f}s...", flush=True)
                time.sleep(wait)
                last_err = exc
                continue
            raise
    raise RuntimeError(f"Failed after {attempts} attempts: {last_err}")


def _save_quantized_png(im: Image.Image, out_path: Path) -> None:
    # 256-color palette quantization without dithering suits flat cel shading
    # (Wave 3 spec §4.3) and cuts embedded PNG weight by ~4.5x with no visible loss.
    q = im.quantize(colors=256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)
    q.save(out_path, format="PNG", optimize=True)


def save_512_and_256(master_path: Path, out_full: Path, out_small: Path) -> None:
    out_full.parent.mkdir(parents=True, exist_ok=True)
    with Image.open(master_path) as im:
        rgb = im.convert("RGB")
        w, h = rgb.size
        # 512x512 full portrait (sRGB, no alpha)
        full = rgb.resize((512, 512), Image.Resampling.LANCZOS)
        _save_quantized_png(full, out_full)
        # 256x256 tight head crop per Wave 3 spec §4.3: 18%..82% W, 6%..70% H
        x0 = round(w * 0.18)
        y0 = round(h * 0.06)
        side = round(w * 0.64)
        crop = rgb.crop((x0, y0, min(w, x0 + side), min(h, y0 + side)))
        small = crop.resize((256, 256), Image.Resampling.LANCZOS)
        _save_quantized_png(small, out_small)


def make_rounded(im: Image.Image, size: int) -> Image.Image:
    r = max(5, min(20, round(size * 0.25)))
    resized = im.convert("RGBA").resize((size, size), Image.Resampling.LANCZOS)
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=r, fill=255)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(resized, (0, 0), mask)
    return out


def build_contact_sheet(
    title: str,
    items: list[tuple[str, str, Path, Path]],
    out_path: Path,
) -> None:
    """Build a dark + light review contact sheet showing 160, 48, 36, 24 pt sizes."""
    cols = 4
    rows = (len(items) + cols - 1) // cols
    cell_w = 310
    cell_h = 250
    pad = 24
    header_h = 56
    w = pad * 2 + cols * cell_w
    h = header_h + pad + rows * cell_h + pad

    sheet = Image.new("RGB", (w, h), (14, 15, 18))
    draw = ImageDraw.Draw(sheet)
    try:
        font_title = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 22)
        font_label = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 14)
        font_sub = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 12)
    except Exception:
        font_title = font_label = font_sub = ImageFont.load_default()

    draw.text((pad, 16), title, fill=(230, 231, 234), font=font_title)

    for idx, (label, subtitle, full_path, small_path) in enumerate(items):
        if not full_path.exists():
            continue
        col = idx % cols
        row = idx // cols
        x = pad + col * cell_w
        y = header_h + row * cell_h

        # Card background split: dark top/left, light strip bottom-right
        draw.rounded_rectangle(
            (x + 4, y + 4, x + cell_w - 8, y + cell_h - 8),
            radius=12,
            fill=(21, 22, 25),
            outline=(45, 47, 54),
        )
        # Light preview strip on right side of card
        draw.rounded_rectangle(
            (x + 186, y + 40, x + cell_w - 18, y + cell_h - 18),
            radius=8,
            fill=(232, 232, 234),
        )

        draw.text((x + 16, y + 12), label, fill=(230, 231, 234), font=font_label)
        draw.text((x + 115, y + 14), subtitle, fill=(150, 156, 167), font=font_sub)

        with Image.open(full_path) as full_im, Image.open(small_path if small_path.exists() else full_path) as small_im:
            # 160px on dark
            p160 = make_rounded(full_im, 160)
            sheet.paste(p160, (x + 16, y + 42), p160)
            # 48, 36, 24 on light strip
            p48 = make_rounded(full_im, 48)
            p36 = make_rounded(full_im, 36)
            p24 = make_rounded(small_im, 24)
            p16 = make_rounded(small_im, 16)
            sheet.paste(p48, (x + 212, y + 52), p48)
            sheet.paste(p36, (x + 218, y + 110), p36)
            sheet.paste(p24, (x + 206, y + 158), p24)
            sheet.paste(p16, (x + 240, y + 162), p16)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out_path, format="PNG", optimize=True)


def generate_one(
    api_key: str,
    spec: CharacterSpec,
    out_path: Path,
    model: str,
    quality: str,
    ref_path: Path | None,
    force: bool = False,
) -> Path:
    if out_path.exists() and not force:
        print(f"  [skip] {out_path.relative_to(ROOT)} already exists", flush=True)
        return out_path
    out_path.parent.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    prompt = build_prompt(spec, with_ref=ref_path is not None)
    if ref_path is not None:
        png = api_edit(api_key, prompt, ref_path, model, quality)
    else:
        png = api_generate(api_key, prompt, model, quality)
    out_path.write_bytes(png)
    dt = time.time() - t0
    print(f"  [ok] {spec.key} -> {out_path.relative_to(ROOT)} ({len(png)//1024} KB, {dt:.1f}s)", flush=True)
    return out_path


def generate_expression(
    api_key: str,
    captain_key: str,
    face: str,
    normal_master: Path,
    out_path: Path,
    model: str,
    quality: str,
    force: bool = False,
) -> Path:
    if out_path.exists() and not force:
        print(f"  [skip] {out_path.relative_to(ROOT)}", flush=True)
        return out_path
    out_path.parent.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    table = BOLT_EXPRESSION_EDITS if captain_key == "bolt" else EXPRESSION_EDITS
    prompt = table[face]
    png = api_edit(api_key, prompt, normal_master, model, quality)
    out_path.write_bytes(png)
    dt = time.time() - t0
    print(f"  [ok] {captain_key}/{face} ({len(png)//1024} KB, {dt:.1f}s)", flush=True)
    return out_path


def build_expressions_sheet(out_path: Path) -> None:
    faces = ("Normal", "Happy", "Determined", "Surprised", "Dizzy")
    tile = 128
    gap = 10
    label_w = 110
    pad = 20
    header_h = 44
    w = pad * 2 + label_w + len(faces) * tile + (len(faces) - 1) * gap
    h = header_h + pad + len(CAPTAINS) * (tile + gap) + pad
    sheet = Image.new("RGB", (w, h), (14, 15, 18))
    draw = ImageDraw.Draw(sheet)
    try:
        font = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 14)
        font_sub = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 12)
    except Exception:
        font = font_sub = ImageFont.load_default()
    for fi, face in enumerate(faces):
        draw.text((pad + label_w + fi * (tile + gap) + 6, 14), face, fill=(180, 185, 195), font=font)
    for row, c in enumerate(CAPTAINS):
        y = header_h + row * (tile + gap)
        draw.text((pad, y + 42), c.name, fill=(230, 231, 234), font=font)
        draw.text((pad, y + 62), c.role, fill=(150, 156, 167), font=font_sub)
        for fi, face in enumerate(faces):
            p = CREW_DIR / c.key / f"{face}.png"
            if not p.exists():
                continue
            with Image.open(p) as im:
                rounded = make_rounded(im, tile)
                sheet.paste(rounded, (pad + label_w + fi * (tile + gap), y), rounded)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out_path, format="PNG", optimize=True)


def install_all() -> None:
    # Install 12 Captains into assets/crew/<key>/
    captain_items = []
    for c in CAPTAINS:
        master_dir = MASTERS_DIR / "captains" / c.key
        normal_master = master_dir / "Normal.png"
        if not normal_master.exists():
            continue
        dest_dir = CREW_DIR / c.key
        dest_dir.mkdir(parents=True, exist_ok=True)
        # Illustrated portraits do not use legacy pixel animation strips (preserved in assets/crew-pixel)
        for strip in dest_dir.glob("*.strip.png"):
            strip.unlink()
        save_512_and_256(normal_master, dest_dir / "Normal.png", dest_dir / "Normal.small.png")
        for face in ("Happy", "Determined", "Surprised", "Dizzy"):
            f_master = master_dir / f"{face}.png"
            if f_master.exists():
                save_512_and_256(f_master, dest_dir / f"{face}.png", dest_dir / f"{face}.small.png")
            else:
                shutil.copyfile(dest_dir / "Normal.png", dest_dir / f"{face}.png")
                shutil.copyfile(dest_dir / "Normal.small.png", dest_dir / f"{face}.small.png")
        # Per Wave 3 spec §4.3: Sad uses the same image as Dizzy
        shutil.copyfile(dest_dir / "Dizzy.png", dest_dir / "Sad.png")
        shutil.copyfile(dest_dir / "Dizzy.small.png", dest_dir / "Sad.small.png")
        captain_items.append((c.name, c.role, dest_dir / "Normal.png", dest_dir / "Normal.small.png"))

    # Install 8 Provider Mascots into assets/providers/crew/
    mascot_items = []
    for m in MASCOTS:
        master = MASTERS_DIR / "mascots" / f"{m.key}.png"
        if not master.exists():
            continue
        out_full = MASCOT_DIR / f"{m.key}.png"
        out_small = MASCOT_DIR / f"{m.key}.small.png"
        save_512_and_256(master, out_full, out_small)
        mascot_items.append((m.key.upper(), m.role, out_full, out_small))

    if captain_items:
        build_contact_sheet(
            "Shina Wave 3 — 12 Illustrated Anime Captains (160 / 48 / 36 / 24 / 16 pt)",
            captain_items,
            ROOT / "artifacts" / "wave3-captains-sheet.png",
        )
        build_expressions_sheet(ROOT / "artifacts" / "wave3-expressions-sheet.png")
    if mascot_items:
        build_contact_sheet(
            "Shina Wave 3 — 8 Provider Crewmate Mascots (160 / 48 / 36 / 24 / 16 pt)",
            mascot_items,
            ROOT / "artifacts" / "wave3-mascots-sheet.png",
        )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--key-file", default=None)
    ap.add_argument("--model", default=DEFAULT_MODEL)
    ap.add_argument("--quality", default=DEFAULT_QUALITY)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--only", nargs="*", default=None, help="Specific keys to generate")
    ap.add_argument(
        "--stage",
        choices=["anchor", "captains", "mascots", "expressions", "install"],
        required=True,
    )
    args = ap.parse_args()

    if args.stage == "install":
        install_all()
        print("Installed assets and generated contact sheets.", flush=True)
        return 0

    api_key = read_api_key(args.key_file)
    anchor_path = MASTERS_DIR / "captains" / "sable" / "Normal.png"

    if args.stage == "anchor":
        sable = next(c for c in CAPTAINS if c.key == "sable")
        generate_one(api_key, sable, anchor_path, args.model, args.quality, ref_path=None, force=args.force)
        return 0

    if not anchor_path.exists():
        raise RuntimeError(f"Style anchor missing: {anchor_path}. Run --stage anchor first.")

    if args.stage == "captains":
        targets = [c for c in CAPTAINS if c.key != "sable"]
        if args.only:
            targets = [c for c in CAPTAINS if c.key in args.only]
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
            futs = [
                pool.submit(
                    generate_one,
                    api_key,
                    c,
                    MASTERS_DIR / "captains" / c.key / "Normal.png",
                    args.model,
                    args.quality,
                    None if c.key == "sable" else anchor_path,
                    args.force,
                )
                for c in targets
            ]
            for fut in concurrent.futures.as_completed(futs):
                fut.result()
        return 0

    if args.stage == "mascots":
        targets = MASCOTS
        if args.only:
            targets = [m for m in MASCOTS if m.key in args.only]
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
            futs = [
                pool.submit(
                    generate_one,
                    api_key,
                    m,
                    MASTERS_DIR / "mascots" / f"{m.key}.png",
                    args.model,
                    args.quality,
                    anchor_path,
                    args.force,
                )
                for m in targets
            ]
            for fut in concurrent.futures.as_completed(futs):
                fut.result()
        return 0

    if args.stage == "expressions":
        targets = CAPTAINS
        if args.only:
            targets = [c for c in CAPTAINS if c.key in args.only]
        jobs = []
        for c in targets:
            normal_master = MASTERS_DIR / "captains" / c.key / "Normal.png"
            if not normal_master.exists():
                raise RuntimeError(f"Missing Normal master for {c.key}")
            for face in ("Happy", "Determined", "Surprised", "Dizzy"):
                jobs.append((c.key, face, normal_master, MASTERS_DIR / "captains" / c.key / f"{face}.png"))
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
            futs = [
                pool.submit(
                    generate_expression,
                    api_key,
                    ck,
                    face,
                    nm,
                    out_p,
                    args.model,
                    args.quality,
                    args.force,
                )
                for ck, face, nm, out_p in jobs
            ]
            for fut in concurrent.futures.as_completed(futs):
                fut.result()
        return 0

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
