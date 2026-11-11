# Shina crew portraits

The twelve captains are original characters drawn as 40x40 pixel-art busts by
[`tools/crew_originals.py`](../../tools/crew_originals.py). The framing follows
Pokémon Mystery Dungeon portraits: a tight bust crop, outlines tinted by the region
they border, three-tone shading, and six expressions per character. Each is
presented on a flat, saturated square tile like the crew avatars in Capy.

| Folder | Captain | Look |
|---|---|---|
| `ember` | Ember, Platform | messy copper hair, goggles on the forehead, olive work jacket, slate tile |
| `juniper` | Juniper, Release | long mint hair with a blunt fringe, star clip, cream cardigan, rose tile |
| `sable` | Sable, Support | indigo bob, coral headphones, grey hoodie, ochre tile |
| `pip` | Pip, Research | white-blonde curls in a top bun, round glasses, green coat, teal tile |
| `bram` | Bram, Operations | white captain's cap, grey moustache, navy peacoat with brass buttons, teal tile |
| `wren` | Wren, Interface | mustard beanie with a pompom, chestnut bob, freckles, striped scarf, plum tile |
| `rook` | Rook, Security | indigo hood, dark fringe, gold eyes, silver clasp, olive tile |
| `nia` | Nia, Data | centre-parted braids with gold cuffs, yellow headband, teal turtleneck, coral tile |
| `jett` | Jett, Performance | pink mohawk, shaved sides, leather jacket, cheek plaster, mustard tile |
| `otto` | Otto, Review | bald, white beard and brows, round glasses, burgundy cardigan, grey tile |
| `bolt` | Bolt, Automation | robot with an antenna, ear bolts and a glowing screen face, cobalt tile |
| `kit` | Kit, Testing | fox ears, spiky russet hair, cream hoodie, blue neckerchief, green tile |

The folder order is the portrait index saved in captain profiles, so new captains are only
ever appended. The first four must stay byte-identical when the set is regenerated.

## Animation strips

Each captain also has four sprite strips that the sidebar plays. Every frame is a full
40x40 bust at the same 6x scale, laid side by side. Frame 0 is always the matching static
face, so a paused portrait or one under reduced motion looks exactly like the static set.
Corner marks stay fixed to the tile while the character moves.

| Strip | Built on | Frames |
|---|---|---|
| `Idle.strip.png` | Normal | rest, head settled 1 px (breathing), eyes half shut, eyes shut |
| `Working.strip.png` | Determined | rest with three dots, one dot lit, two dots lit with a 1 px nod, blink |
| `Attention.strip.png` | Surprised | rest, 2 px hop with "!", 1 px hop with "!", rest with "!" |
| `Happy.strip.png` | Happy | rest, 2 px hop with a sparkle, 1 px hop with a sparkle |

The timing lives in `Clip::timeline` in [`src/visuals.rs`](../../src/visuals.rs): Idle
breathes about every three seconds and blinks once per seven-second loop, Working steps about
four times a second, Attention hops twice and then holds for about five seconds, and Happy
hops twice and stops. Keep the frame order in the generator and the timeline in step.

Regenerate the set with:

```powershell
python -I tools/crew_originals.py assets/crew
```

To review the whole crew, write a sheet with every face at 80 px and the Normal face at the
sidebar's 44, 36 and 24 px sizes into the ignored `artifacts` folder:

```powershell
python -I tools/crew_originals.py --sheet artifacts/crew-sheet.png
```

To use a different set, see the drop-in instructions in the main README.
