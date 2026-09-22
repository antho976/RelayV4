# GDD, One-Page Design and Feature Spec Templates

Keep design docs as Markdown in the repository (for example `Docs/Design/`), so they diff and
review like code. A GDD is a *living* index of decisions, not a novel. Each section should link
to the asset or code that implements it once it exists.

Three document sizes:
- **One-page design**: pitch a game or a big feature on one page. Write it first.
- **GDD**: the whole game, organized by system, updated as decisions are made.
- **Feature spec**: one mechanic or system, detailed enough to implement and test.

---

## One-page design

```markdown
# <Working Title>: One-Page Design

**Fantasy:** <one sentence: who the player is and what they do>
**Genre / references:** <genre>; plays like <A> meets <B>
**Platform / input:** <PC, console, mobile>; <gamepad-first / KBM / touch>
**Session length:** <typical play session>   **Total length:** <hours>

## Pillars
1. <Pillar>: rules out <what it forbids>
2. ...

## Player verbs
<verb>, <verb>, <verb> (primary) | <verb>, <verb> (secondary)

## Loops
- Core (seconds): <A> → <B> → <C> → <A>
- Session (minutes): ...
- Meta (hours): ...   Feeds from core: <currency / XP / unlocks / story flags>

## Hook
<why this is interesting; the one thing nobody else does>

## Biggest risks (prototype these first)
1. <risk>: prototype: <what we build to answer it>, by <date>
```

---

## GDD skeleton

```markdown
# <Title>: Game Design Document
Status: <draft / living>   Owner: <name>   Last updated: <date>

## 1. Overview
Fantasy, pillars, target audience, platforms, comparable titles, unique selling points.

## 2. Loops and structure
Core / session / meta loop diagrams. Game structure (levels, hub, open world, runs).
Win / lose / fail states. What persists between sessions.

## 3. Player character
Verbs and their inputs (table: Verb | Input (gamepad / KBM) | Input Action asset | Notes).
Movement metrics: walk/run speed, jump height and distance, crouch height, capsule size.
(Point to the character Blueprint / Data Asset holding these; see level-metrics in unreal-level-environment.)
Camera: perspective, FOV, lag, collision behavior.

## 4. Mechanics (one subsection per system)
For each: purpose (which pillar/loop it serves), rules, player-facing feedback,
tunables (and the asset that holds them), edge cases, dependencies.
- Combat / abilities (GAS? see unreal-gas)
- Health, damage, death, respawn / checkpoints
- Inventory / items / equipment
- Progression (XP, levels, skill trees, unlocks)
- Economy (currencies, sources, sinks)
- AI / enemies (archetypes table: Name | Role | Tell | Counter | Definition asset)
- Interaction, puzzles, traversal

## 5. Content
World / level list (table: Level | Purpose | New mechanic introduced | Length | Map asset).
Enemy, item, ability catalogs → link to the Data Assets / Data Tables, not copies of numbers.
Narrative summary → link to narrative docs (unreal-narrative).

## 6. Progression and difficulty
Intended difficulty curve (per level tension graph), mechanic introduction order,
difficulty presets (Data Asset per preset), assist options.

## 7. Data and tags
Gameplay Tag namespaces owned by this game (Ability.*, Status.*, Item.*, Story.*, Event.*).
Primary Asset types (WeaponDefinition, EnemyDefinition, ...) and their folders.
Curve Tables and Data Tables and their CSV sources.

## 8. UI / UX
Screens flow (boot → title → main menu → gameplay → pause → results).
HUD elements and the information each communicates. Controller navigation.

## 9. Audio and feel
Feedback per verb (sound / VFX / shake / rumble / hit-stop). Music states.

## 10. Accessibility
Remapping, subtitles, colorblind considerations, difficulty/assists, motion options, text size.

## 11. Technical constraints
Engine version, target hardware and frame rate, multiplayer model, save system, streaming (World Partition?),
platform certification notes.

## 12. Telemetry and playtests
Events logged, key metrics, playtest schedule and questions.

## 13. Scope
Milestones with exit criteria. Cut list (ranked). Open questions / decisions log.
```

---

## Feature spec (one mechanic)

```markdown
# Feature: <Name>
Pillar(s) served: <..>   Loop: <core/session/meta>   Priority: <must/should/could>
Owner: <name>   Status: <proposed / prototyping / in production / done / cut>

## Player experience
What the player feels and does, in 2–3 sentences. The "fantasy" of this feature.

## Rules
Numbered, testable statements. ("Dash moves the character 600 cm over 0.2 s; grants
invulnerability for the first 0.15 s; costs 1 charge; charges recharge 1 per 2 s, max 2.")

## Inputs and states
Input Action(s), buffering window, what cancels it, what it cancels.
State diagram if more than two states.

## Feedback
Animation, VFX, SFX, camera, rumble, UI (charge pips), accessibility alternatives.

## Tunables
| Name | Default | Range | Lives in |
|---|---|---|---|
| DashDistance | 600 | 300–900 | DA_Dash (UDashDefinition) |

## Gameplay Tags
New tags and meaning (e.g. Ability.Dash, State.Invulnerable, Cooldown.Dash).

## Implementation notes
Classes / Blueprints / assets to create; C++ vs Blueprint split; networking; save data.

## Edge cases
Dash into wall, off ledge, during hit reaction, while rooted, at 0 charges, on moving platform.

## Acceptance / test plan
How a tester verifies each rule. Metrics to log. Prototype question it answers.

## Cut / fallback
Simplest version worth shipping if time runs out.
```

---

## Decision log entry

```markdown
### <YYYY-MM-DD>: <decision>
Context: <what question came up>
Options: <A>, <B>, <C>
Decision: <chosen> because <reason tied to a pillar / playtest data>
Consequences: <what changes, what to revisit>
```

## Writing guidance for agents

- Write numbers once. In the doc, *name* the tunable and link the asset. If you must quote a value,
  mark it "(initial)" so readers know the asset is authoritative.
- Every mechanic section needs **tunables**, **feedback** and **edge cases**. Those are the parts
  people most often leave out.
- Prefer tables and numbered rules to prose.
- When you implement a spec, update its Status and fill in "Lives in" with real asset paths
  (use `ue_search_assets` to confirm the path).
- Keep a "Questions for the human" list at the bottom rather than inventing answers to design questions.
