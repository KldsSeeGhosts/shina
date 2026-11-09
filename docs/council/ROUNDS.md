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
