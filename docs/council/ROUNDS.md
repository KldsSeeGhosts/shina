# Design council ratings

Each round, three judges score Shina from rendered screenshots and the source: Gemini 4 Argon, GPT 6.1 Sol (medium reasoning), and Claude Sonnet 5.5 (high reasoning). Claude Opus 5.5 orchestrates, picks which recommendations ship, and has the final say. Gemini ran on medium reasoning through Antigravity in rounds 1-2 and on low reasoning through Pi from round 2 onward.

Every judge scores the baseline, the previous round, the current round, and a live Capy window on one 0-10 scale, so each table is a comparison against a fixed baseline. The brief the judges follow is in [BRIEF.md](BRIEF.md). Screenshots for each round are in the `rN/` folders, rendered headlessly with `--demo --snap`.

Scores from different judges are not on the same scale. Compare a judge with itself across rounds, or use the averages.

## Overall, averaged across judges

| Round | Shina | Capy | Crew art | Verdict |
|---|---|---|---|---|
| 0 (baseline) | 6.2 | | | |
| 2 | 7.5 | 8.5 | | 3 of 3 approve |
| 3 | 7.8 | 8.5 | 7.6 (Capy 7.7) | 3 of 3 approve |
| 4 | 7.9 | 8.7 | 8.1 (Capy 7.7) | 3 of 3 approve |
| 5 | 8.1 | 8.7 | 8.1 (Capy 7.9) | 3 of 3 approve |
| 6 | 8.1 | 8.6 | 8.3 (Capy 7.8) | 3 of 3 approve |
| 7 | 8.0 | 8.6 | 7.7 (Capy 7.7) | 3 of 3 approve |
| 8 | 8.1 | 8.8 | 8.0 (Capy 8.1) | 3 of 3 approve |
| 9 | 8.3 | 9.0 | 8.2 (Capy 8.5) | 3 of 3 approve |
| 10 | 8.4 | 8.9 | 8.3 (Capy 8.2) | 3 of 3 approve |
| 11 | 8.4 | 9.0 | 8.4 (Capy 8.0) | 2 of 3 approve |
| 12 | 8.6 | 9.0 | 8.4 (Capy 8.4) | 3 of 3 approve |

## Round 1

Judges scored the baseline only, against Capy, T3 Code, Codex, and Claude Code as references.

| Judge | Overall |
|---|---|
| Sonnet 5.5 | 5.8 |
| GPT 6.1 Sol | 6.9 |
| Gemini 4 Argon | 7.1 |

Shipped in round 2: one demo badge instead of four "Example" labels; truthful captain status marks instead of a padlock; distinct glyphs for threads, issues, automations, activity, and theme; dimmed unbuilt navigation; suggestion chips and disclosure pills; darker small text; a conversation column aligned with the composer; wider narrow-window tabs; darker dialog backdrops.

## Round 2 (`r2/`)

| Judge | Baseline | Round 2 | Capy | Approves |
|---|---|---|---|---|
| Sonnet 5.5 | 6.5 | 7.2 | 8.1 | yes |
| GPT 6.1 Sol | 6.4 | 7.2 | 8.5 | yes |
| Gemini 4 Argon | 7.1 | 8.0 | 8.9 | yes |

Shipped in round 3: original captains (Ember, Juniper, Sable, Pip) as the default portraits; rename and edit from the captain row and the picker; quiet disclosure lines; single-line thread rows; a solid primary button and a close button in the captain editor; a raised active tab; a charcoal dark theme and lighter body weight; a larger send button; an auto-collapsing sidebar below 760 px.

## Round 3 (`r3/`)

| Judge | Baseline | Round 2 | Round 3 | Capy | Crew art r2 / r3 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.5 | 6.7 | 7.4 | 8.1 | 5.5 / 7.0 / 6.5 | yes |
| GPT 6.1 Sol | 6.4 | 7.2 | 7.8 | 8.6 | 7.0 / 7.8 / 8.0 | yes |
| Gemini 4 Argon | 6.8 | 7.7 | 8.3 | 8.9 | 6.3 / 8.0 / 8.6 | yes |

Shipped in round 4: demo data whose "Working" state comes from a running thread; dots only for unread or live work; an asleep Offline face; head-crop portraits at 24 px and below; narrower back hair and adjusted tiles; active state for Threads and Archive; filled form fields; no duplicate header pencil; tighter reply rhythm.

## Round 4 (`r4/`)

| Judge | Baseline | Round 3 | Round 4 | Capy | Crew art r3 / r4 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.8 | 7.1 | 7.4 | 8.4 | 6.6 / 7.4 / 6.0 | yes |
| GPT 6.1 Sol | 6.5 | 7.7 | 8.0 | 8.7 | 7.5 / 8.1 / 8.3 | yes |
| Gemini 4 Argon | 6.9 | 8.0 | 8.3 | 9.1 | 7.9 / 8.8 / 8.7 | yes |

Shipped in round 5: unbuilt Issues, Pull requests, and Automations removed; Threads highlighted only without an open captain; tab portrait matches the captain's status; captain home names a working thread; composer dropdowns keep their chevron; "Medium thinking" labels; no redundant "Thread settled" footer; lighter form fields and portrait selection; a light logo badge in the dark theme; a focused Working face, warmer Pip hair, and a tighter head crop.

## Round 5 (`r5/`)

| Judge | Baseline | Round 4 | Round 5 | Capy | Crew art r4 / r5 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 6.4 | 7.4 | 7.8 | 8.3 | 7.3 / 7.5 / 6.5 | yes |
| GPT 6.1 Sol | 6.2 | 7.7 | 8.0 | 8.7 | 7.8 / 8.2 / 8.3 | yes |
| Gemini 4 Argon | 7.0 | 8.2 | 8.5 | 9.0 | 8.1 / 8.5 / 8.8 | yes |

Round 5 regressed one thing: the tighter head crop cuts through the new "..." and "z" corner marks, so a Working captain looks like a Ready one at sidebar size. Round 6 fixes it.

Shipped in round 6: status badges on small avatars with clean head crops; an alert mark for Needs you; one sidebar selection and a detached nested-thread guide line; grid and sun/moon icons; truthful picker wording and a separate working-thread line; quieter disclosures, visible field borders, a raised dark user bubble; a demo reply with a list and inline code.

## Round 6 (`r6/`)

| Judge | Baseline | Round 5 | Round 6 | Capy | Crew art r5 / r6 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.4 | 7.0 | 7.4 | 8.2 | 7.3 / 7.7 / 6.5 | yes |
| GPT 6.1 Sol | 6.6 | 8.0 | 8.2 | 8.7 | 8.0 / 8.3 / 8.0 | yes |
| Gemini 4 Argon | 7.1 | 8.4 | 8.6 | 9.0 | 8.6 / 8.8 / 8.9 | yes |
| **Average** | 6.4 | 7.8 | **8.1** | 8.6 | 8.0 / **8.3** / 7.8 | 3 of 3 |

All three judges flagged the same remaining issue: captain rows show status twice, as an avatar badge and a trailing mark. Round 7 keeps only the badge.

Shipped in round 7: one status mark per captain row; dot, ring, and square badge shapes; tab shows the open conversation's status; three-line captain home; one captain order everywhere; 16 px toolbar glyphs, borderless inline code, lighter dark body text, a darker dark sidebar, and a crisper light user bubble.

## Round 7 (`r7/`)

| Judge | Baseline | Round 6 | Round 7 | Capy | Crew art r6 / r7 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.5 | 7.2 | 7.5 | 8.2 | 7.0 / 7.0 / 6.0 | yes |
| GPT 6.1 Sol | 6.2 | 7.7 | 7.9 | 8.5 | 7.7 / 7.7 / 8.0 | yes |
| Gemini 4 Argon | 6.8 | 8.5 | 8.7 | 9.2 | 8.4 / 8.4 / 9.0 | yes |
| **Average** | 6.2 | 7.8 | **8.0** | 8.6 | 7.7 / **7.7** / 7.7 | 3 of 3 |

Crew art was unchanged this round. Judges asked for larger badges whose shapes read at 1x, one status vocabulary across badges, chips, and thread rows, and a captain home whose status agrees with the sidebar.

Shipped in round 8: a single top strip for sidebar header, tabs, actions, and caption buttons; one status vocabulary in badges, chips, rows, and headers; badges at every avatar size; a captain home showing the captain's overall status; brighter dark status text; a demo composer placeholder; a clearer selected portrait; a whole-window review brief with native renders and corner crops.

## Round 8 (`r8/`)

| Judge | Baseline | Round 7 | Round 8 | Capy | Crew art r7 / r8 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.0 | 6.7 | 7.4 | 8.1 | 7.3 / 7.3 / 7.0 | yes |
| GPT 6.1 Sol | 6.3 | 7.7 | 8.2 | 8.8 | 8.0 / 8.0 / 8.1 | yes |
| Gemini 4 Argon | 6.3 | 7.9 | 8.7 | 9.4 | 8.8 / 8.8 / 9.2 | yes |
| **Average** | 5.9 | 7.4 | **8.1** | 8.8 | 8.0 / **8.0** / 8.1 | 3 of 3 |

First round under the whole-window brief. Window chrome & alignment, round 7 to round 8: Sonnet 4.5 to 7.2, GPT 6.4 to 8.8, Gemini 6.4 to 8.9. Whole window: Sonnet 6.0 to 7.4, GPT 7.4 to 8.2, Gemini 7.2 to 8.7. Judges now flag the narrow reading column at native size and small alignment drifts along the sidebar's right edge.

Shipped in round 9: an 832px measure with 14px body text on wide windows, a tab aligned to the pane edge, one sidebar alignment grid, a divider before the caption buttons, a separated light-theme pane, visibly disabled demo controls, a single captain-home status, and a distinct activity glyph.

## Round 9 (`r9/`)

| Judge | Baseline | Round 8 | Round 9 | Capy | Crew art r8 / r9 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.4 | 7.3 | 7.7 | 8.6 | 7.6 / 7.6 / 8.0 | yes |
| GPT 6.1 Sol | 6.2 | 8.1 | 8.4 | 8.9 | 8.3 / 8.3 / 8.2 | yes |
| Gemini 4 Argon | 6.1 | 8.5 | 8.8 | 9.4 | 8.8 / 8.8 / 9.2 | yes |
| **Average** | 5.9 | 8.0 | **8.3** | 9.0 | 8.2 / **8.2** / 8.5 | 3 of 3 |

Window chrome & alignment, round 8 to round 9: Sonnet 7.0 to 7.8, GPT 8.0 to 8.7, Gemini 8.7 to 9.0. No judge found a pixel-level chrome break. Open items: sidebar text starts drift by a few pixels, user bubbles stay 13px on wide windows, hidden header buttons leave phantom gaps, Juniper's Needs you state is easy to miss, and the light theme's composer border is faint.

Shipped in round 10: header actions without phantom gaps and with on states, a balanced caption divider, one sidebar text column, a Needs you label, 14px user prompts on wide windows, 12px small text, firmer light edges, readable locked settings, portrait names that describe the art, and per-size avatar keys.

## Round 10 (`r10/`)

| Judge | Baseline | Round 9 | Round 10 | Capy | Crew art r9 / r10 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.4 | 7.4 | 7.9 | 8.5 | 7.8 / 7.8 / 7.0 | yes |
| GPT 6.1 Sol | 6.3 | 8.1 | 8.3 | 8.8 | 8.3 / 8.3 / 8.5 | yes |
| Gemini 4 Argon | 6.3 | 8.8 | 9.1 | 9.4 | 8.7 / 8.7 / 9.2 | yes |
| **Average** | 6.0 | 8.1 | **8.4** | 8.9 | 8.3 / **8.3** / 8.2 | 3 of 3 |

Consistency & polish, round 9 to round 10: Sonnet 7.3 to 7.9, GPT 8.2 to 8.4, Gemini 8.7 to 9.1. Judges split on portrait names (Gemini and GPT prefer the new look names, Sonnet finds them confusing). Open items: empty space between transcript and composer, chevrons on disclosures that open the activity pane, the Threads nav never shows active, the captain home composer floats mid-pane, and dark surfaces separate faintly.

Shipped in round 11: right-pointing disclosure chevrons that open the activity pane, muted settled text, 400-weight dark body text, firmer dark hairlines, theme-fit modal scrims, recessed light form fields, and a narrow composer on the transcript edge.

## Round 11 (`r11/`)

| Judge | Baseline | Round 10 | Round 11 | Capy | Crew art r10 / r11 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 5.4 | 7.4 | 7.7 | 8.6 | 7.8 / 7.8 / 7.0 | yes |
| GPT 6.1 Sol | 6.4 | 8.3 | 8.4 | 9.0 | 8.5 / 8.5 / 8.3 | no |
| Gemini 4 Argon | 6.1 | 8.9 | 9.1 | 9.3 | 8.9 / 8.9 / 8.8 | yes |
| **Average** | 6.0 | 8.2 | **8.4** | 9.0 | 8.4 / **8.4** / 8.0 | 2 of 3 |

GPT found the gain too small to count as a whole-product advance; the orchestrator merged on the strength of the corrections. All three flag Sable's sidebar check, still teal while Settled is muted elsewhere. Open items: the native-width column leaves a large empty pane, the sidebar footer rule runs past the row column, the captain row stays selected while a nested thread is open, and demo suggestion pills look enabled.

Shipped in round 12: one muted settled check, captain selection only on the captain home, a contained sidebar footer rule, an 896px native measure, a firmer dark composer and send button, 0.28 light scrims, no placeholder model label, and disabled demo suggestions.

## Round 12 (`r12/`)

| Judge | Baseline | Round 11 | Round 12 | Capy | Crew art r11 / r12 / Capy | Approves |
|---|---|---|---|---|---|---|
| Sonnet 5.5 | 6.0 | 8.0 | 8.2 | 8.5 | 8.0 / 8.0 / 7.5 | yes |
| GPT 6.1 Sol | 6.9 | 8.4 | 8.6 | 9.0 | 8.3 / 8.3 / 8.5 | yes |
| Gemini 4 Argon | 6.2 | 8.6 | 8.9 | 9.4 | 8.9 / 8.9 / 9.2 | yes |
| **Average** | 6.4 | 8.3 | **8.6** | 9.0 | 8.4 / **8.4** / 8.4 | 3 of 3 |

Gains are now about 0.2 a round; the remaining gaps are structural. All three ask for wide-window code and heading sizes to scale with the 14px body. Gemini finds the 50% demo suggestion pills washed out, while GPT approves them. Open items: an in-pane header, relative times on sidebar rows, the nested selection touching the guide line, dark tonal steps, and a demo composer that still accepts typing.
