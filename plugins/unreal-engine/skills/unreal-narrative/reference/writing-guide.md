# Game Writing Guide: Lines, Subtitles, Barks, Bibles, Localization

## 1. Principles

- **Players are busy.** They read while moving, fighting and looking elsewhere. Every line has to survive
  being half-heard. Put the key information first ("The bell. Ring it before the tide turns.").
- **One line, one idea.** Split long speeches into several nodes, each 1–2 sentences.
- **Show through play.** If a mechanic or place can demonstrate something, cut the line that explains it.
- **No exposition dumps.** Spread lore across barks, readables and environment. Critical story is stated plainly
  at least twice, in different forms (a line and an objective text).
- **Voice over vocabulary.** Characters differ through rhythm, what they notice, and what they avoid saying,
  not through quirky spelling. Read a line without the speaker name, and you should still know who said it.
- **Respect agency.** Don't make the player character state feelings that the player may not share,
  unless the protagonist is a defined character. Choice text states intent ("[Lie] Say you found nothing").
- **Consistency**: one term per concept (never "Bell Tower" in one place and "Belfry" in another). Keep a glossary in the repo.

## 2. Length limits

| Text | Limit | Notes |
|---|---|---|
| Subtitle line | ≤ 42 characters per line, ≤ 2 lines | Split the node or the subtitle cue if longer |
| Subtitle reading rate | ≤ about 17 characters/second (about 15 for younger audiences) | Minimum display about 1.5 s |
| Dialogue node (spoken) | 1–2 sentences, ≤ about 8 s of VO | Longer lines should be split into nodes |
| Dialogue choice | ≤ about 40 characters | Intent, not script. Add tone tags if needed |
| Objective | ≤ about 40 characters, verb first | "Find the harbor master" |
| Button or menu label | ≤ about 20 characters (English) | Leave room for +30% expansion |
| Tooltip or item description | 1–2 sentences | Mechanics first, flavor second |
| Combat bark | 2–6 words | Must be understood in under 1 s |
| Readable note | ≤ about 120 words | Title plus 1–3 short paragraphs |

The display duration for text without VO is `max(1.5 s, characters / 15)`. With VO, use the audio duration, and
split subtitle cues if the text exceeds 2 lines.

## 3. Subtitle presentation (accessibility)

- On by default, or asked at first launch. Options for size (at least 3 steps), a background box with adjustable
  opacity, and a speaker-name toggle.
- Show the speaker name (and color, but never color alone). Off-screen speakers get a direction or name cue.
- Closed captions (optional setting): important sounds in brackets, `[Bell tolls in the distance]`.
- Do not censor or paraphrase VO in subtitles. Subtitles match the spoken words.
- Keep subtitles in a safe area, never over important gameplay, and readable at 1080p from a couch.

## 4. Barks

- Write **3–6 variants per context** for frequent situations (reload, spotted player, lost target), and 1–2 for rare ones.
- Each variant must work in isolation and in any order. No "as I said before".
- Contexts to cover in combat games: spotted, lost, searching, reloading, hurt, ally down, grenade, flanking, idle
  chatter, and the player doing something unusual (reactivity sells the world).
- Tie some barks to story flags ("Heard the bell rang again...") so the world acknowledges progress.
- Give every bark a priority (callouts outrank chatter) and a cooldown. Nothing repeats within about 30–60 s.

## 5. Script format (spreadsheet or CSV, one row per line)

```csv
LineID,Speaker,Text,Context,Direction,Condition,Next,Notes
Mira_Harbor_010,Character.Mira,"You came. Good. The bell hasn't rung in nine years.",First meeting at the pier,"Relieved, tired",,Mira_Harbor_020,
Mira_Harbor_020,Character.Mira,"Ring it, and the drowned will listen. Maybe.",,"Half-joking, then serious",,Mira_Harbor_Hub,
Mira_Harbor_Hub,Character.Mira,"What do you need?",Hub,Neutral,,,Choices: Ask_Bell / Ask_Rope / Leave
```

- **LineID** = the node ID = the VO file name (`Mira_Harbor_010.wav`). Use increments of 10, so lines can be inserted.
  Never reuse an ID for different text, because VO, localization keys and save data depend on it.
- **Direction** is for the actor and never shown to players. **Context** helps both actors and translators.
- Once lines are recorded, a text change means re-recording. Mark such changes clearly (`Notes: TEXT CHANGED after VO`).
- Import linear lines into a Data Table (see `dialogue-system.md` §4). Branching structure goes into the
  `UDialogueAsset` using the same IDs.

## 6. Localization-aware writing

- **Never assemble sentences from fragments.** `"You found " + Count + " " + Item` can't be translated.
  Write full sentences with named placeholders: `"You found {Count} {Item}"`, and use `FText::Format`.
- **Plurals and gender**: use FText's argument modifiers rather than branching in code:
  `"{Count}|plural(one=rope,other=ropes)"` and `"{Gender}|gender(he,she)"`. Translators adapt them per language.
- **Numbers, dates, currency**: format with `FText::AsNumber`, `FText::AsPercent`, `FText::AsDate` and `FText::AsCurrencyBase`,
  never by hand.
- **Idioms, puns, wordplay and rhymes** don't survive translation. Avoid them in critical text, or add a translator note
  explaining the intent (the *Comment* column of a String Table CSV, or metadata in the Localization Dashboard's
  exported PO files).
- **Text expansion**: German, French, Russian and Portuguese run 20–35% longer. Finnish and German have long compound
  words. CJK text is shorter but needs larger font sizes. UI must wrap or auto-size, so test it with a pseudo-localized
  culture. The Localization Dashboard can generate a pseudo-localization preview. Check the Region & Language
  preview options in your version.
- **Names and terms**: keep proper names in a glossary, and decide per term whether it gets translated.
- **Keys are forever**: changing the *source text* of an existing key marks the translations stale (good), while changing the
  *key* orphans them (bad). Keep keys stable.
- **Fonts**: make sure the font assets cover every shipped script (a Composite Font with fallback sub-fonts for CJK,
  Cyrillic and so on).

## 7. Character bible template

```markdown
# <Name>   (tag: Character.<Name>)
Role in story: <what they do in the plot>        Role in play: <quest giver / vendor / companion / boss>
One-line essence: <"A harbor master who stayed when everyone else fled.">
Want (external goal): <..>        Need (internal lesson): <..>
Wound / secret: <what shaped them; what they hide>
Voice: vocabulary <plain / ornate / technical>; rhythm <short clipped / rambling>;
       verbal habits <..>; never says <..>; humor <dry / none / goofy>
Sample lines (3): <greeting> / <under pressure> / <moment of warmth>
Relationships: <Name>: <how they feel and why>, ...
Arc by chapter: Ch1 <..> → Ch2 <..> → Ch3 <..>
Visual notes: <silhouette, colors, props>        Subtitle color: <#hex, checked for contrast>
VO casting: <age range, accent, reference performances>
Data: DA_Character_<Name> (display name, portrait, subtitle color)
```

## 8. Script linting (plain Python, run on the CSV)

```python
import csv, sys
MAX_LINE, MAX_LINES, CPS = 42, 2, 17.0

def wrap(text, width=MAX_LINE):
    lines, cur = [], ""
    for word in text.split():
        cand = (cur + " " + word).strip()
        if len(cand) > width and cur:
            lines.append(cur); cur = word
        else:
            cur = cand
    return lines + ([cur] if cur else [])

with open(sys.argv[1], newline="", encoding="utf-8") as f:
    ids = set()
    for row in csv.DictReader(f):
        lid, text = row["LineID"], row["Text"]
        if lid in ids: print(f"{lid}: duplicate LineID")
        ids.add(lid)
        n = len(wrap(text))
        if n > MAX_LINES: print(f"{lid}: {n} subtitle lines (> {MAX_LINES}); split the node")
        if len(text) / CPS > 8.0: print(f"{lid}: about {len(text)/CPS:.1f}s to read; consider splitting")
        if "  " in text or text != text.strip(): print(f"{lid}: whitespace issue")
```

Run it with `python lint_script.py Content/Data/Source/Dialogue_Harbor.csv` before importing.

## 9. Review checklist for any new text

- [ ] Key information comes first. Each line holds one idea and fits the length table.
- [ ] Every line has a stable ID, and the speaker is a `Character.*` tag.
- [ ] No sentence fragments are concatenated in code. Placeholders are named, and plurals and gender use FText modifiers.
- [ ] Idioms and puns are avoided or annotated for translators. Glossary terms are used consistently.
- [ ] Barks have enough variants, cooldowns and priorities.
- [ ] Choices state intent, are distinct, and number 2–4.
- [ ] Critical story is also conveyed outside optional text (objective, cutscene, or environment).
