# Shina crew portraits

The four captains are original characters drawn as 40x40 pixel-art busts by
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

Regenerate the set with:

```powershell
python -I tools/crew_originals.py assets/crew
```

To use a different set, see the drop-in instructions in the main README.
