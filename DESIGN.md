---
name: Marvyr
description: Tipografia do Porto. The HUD is the harbor's printed matter, paper slips pinned over the living sea.
colors:
  rag-paper: "#e9dcc0"
  aged-paper: "#d8c7a2"
  press-ink: "#16202b"
  thin-ink: "#4a4f54"
  vermilion: "#c2362b"
  vermilion-ink: "#a8281f"
  sea-teal: "#1f5f73"
  brass: "#d9a441"
  brass-ink: "#7a4e0e"
  ochre-ink: "#9a5a0c"
  brass-wash: "#e7c374"
  bar-track: "rgba(22, 32, 43, 0.16)"
  paper-shadow: "rgba(5, 13, 23, 0.45)"
typography:
  display:
    fontFamily: "Alfa Slab One"
    fontSize: "76px"
    fontWeight: 400
    lineHeight: 1
  headline:
    fontFamily: "Alfa Slab One"
    fontSize: "40px"
    fontWeight: 400
    lineHeight: 1
  title:
    fontFamily: "Alfa Slab One"
    fontSize: "22px"
    fontWeight: 400
    lineHeight: 1.1
  body:
    fontFamily: "Fira Sans"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: 1.3
  action:
    fontFamily: "Fira Sans"
    fontSize: "17px"
    fontWeight: 600
    lineHeight: 1.2
  label:
    fontFamily: "Fira Sans"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: 1.2
  default:
    fontFamily: "Fira Sans"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: 1.2
rounded:
  slip: "2px"
  keycap: "3px"
  button: "4px"
spacing:
  hairline: "2px"
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "12px"
  margin: "16px"
components:
  slip:
    backgroundColor: "{colors.rag-paper}"
    textColor: "{colors.press-ink}"
    rounded: "{rounded.slip}"
    padding: "8px 12px"
  button:
    backgroundColor: "{colors.aged-paper}"
    textColor: "{colors.press-ink}"
    typography: "{typography.action}"
    rounded: "{rounded.button}"
    padding: "5px 10px"
  button-primary:
    backgroundColor: "{colors.brass-wash}"
    textColor: "{colors.press-ink}"
    typography: "{typography.action}"
    rounded: "{rounded.button}"
    padding: "8px 14px"
  button-pressed:
    backgroundColor: "{colors.brass}"
    textColor: "{colors.press-ink}"
  keycap:
    backgroundColor: "{colors.aged-paper}"
    textColor: "{colors.press-ink}"
    rounded: "{rounded.keycap}"
    padding: "2px 7px"
    width: "30px"
  stamp:
    textColor: "{colors.sea-teal}"
    typography: "{typography.label}"
    rounded: "{rounded.slip}"
    padding: "1px 6px"
  field:
    backgroundColor: "{colors.aged-paper}"
    textColor: "{colors.press-ink}"
    padding: "5px 6px"
    height: "34px"
  bar:
    backgroundColor: "{colors.bar-track}"
    rounded: "{rounded.keycap}"
    height: "8px"
---

# Design System: Marvyr

Native Bevy 0.15 client. All values are logical pixels at the 1280x720 reference; `UiScale` multiplies them by `min(w/1280, h/720)` clamped to 0.85..1.6. Tokens live as `const` colors, font handles and bundle helpers in `crates/client/src/ui.rs`, the single source of truth.

## Overview

**Creative North Star: "Tipografia do Porto"**

The HUD is the harbor's printed matter: notices, bounties and manifests that sail with you. Each element is a slip of rag paper pinned over the living sea. It is printed in press ink with a second pass of vermilion, and the headlines are set in wood type. The sea is the stage and fills the screen. Slips stay small and sit at the corners and bottom edge, so the ship and water stay readable between them.

The world rejects the genre default of dark translucent panels with metal borders. Depth is paper lifted off water by a soft shadow. Hierarchy comes from wood type against slab text and from the two-color press, never from glow or gradient. Every visual is drawn in code (Bevy UI nodes and gizmos) over the game's existing sprites. The project uses no generated imagery.

**Key Characteristics:**
- Opaque paper slips, 2px ink border, near-square corners, soft downward shadow.
- Two-color press: ink for everything, vermilion only for danger, stamps and the misregistered masthead.
- Wood-type display (Alfa Slab One) over a clean humanist sans (Fira Sans) in three weights. The slab text face (Zilla Slab) was dropped in v63: at 12-14px it blurred and tired the eye.
- Keycaps printed inline wherever an action is named, swapped per input device.
- Woodcut double rules under mastheads and slip headers.

## Colors

A warm paper-and-ink press palette. The only cool colors are the teal of safe water and the sea itself.

### Primary
- **Press Ink** (press-ink): every border, rule, body text and keycap face. At 50-62% alpha it is also the modal scrim over the sea.
- **Vermilion** (vermilion): the second press color. Used as the offset masthead layer, the PvP warning border, the focused login field underline and large danger type. Never small text.

### Secondary
- **Sea Teal** (sea-teal): safe water and ready state. Used for the PROTEGIDO zone stamp, cannon PRONTO, bar fills and the FEITO step stamp.
- **Brass** (brass): gold as a fill. It is the pressed-button color and is never used for text on paper.

### Tertiary
- **Vermilion Ink** (vermilion-ink): vermilion darkened to reach 4.5:1 on paper, for small danger text and warnings.
- **Brass Ink** (brass-ink): gold amounts as text (the ship panel's gold figure).
- **Ochre Ink** (ochre-ink): intermediate warning, such as frontier water or a half-damaged hull.

### Neutral
- **Rag Paper** (rag-paper): the ground of every slip, poster and pamphlet.
- **Aged Paper** (aged-paper): keycap faces, button rest state, input fields at 60%, alternating rows and headers.
- **Brass Wash** (brass-wash): the selected or primary button, hover blend target (55% mix).
- **Thin Ink** (thin-ink): secondary text on paper, at 4.5:1 or better.
- **Bar Track** (bar-track): ink at 16% alpha behind bar fills.

### Named Rules
**The Two-Pass Press Rule.** Only two inks ever touch the paper: press ink and vermilion, plus their darkened text variants. Teal, brass and ochre are state colors and must carry meaning.

**The Ink-Variant Rule.** A fill color never sets small text on paper. For text, use its `-ink` variant (vermilion-ink, brass-ink, ochre-ink).

## Typography

**Display Font:** Alfa Slab One (OFL, embedded)
**Body Font:** Fira Sans Regular / Medium / SemiBold (OFL, embedded). Medium replaces Bevy's default font, so world labels and signs get accents too.

**Character:** Heavy wood type for the one loud moment, over a sturdy, bookish slab that stays legible at 12-13px on paper.

### Hierarchy
- **Display** (Alfa Slab One, 76px): the MARVYR title on the login poster only.
- **Headline** (Alfa Slab One, 30-40px): mastheads such as the welcome poster (40), graduation (38), help pamphlet (34), zone banner (34) and sea-event and PvP headlines (30).
- **Title** (Alfa Slab One, 19-28px): slip titles, including the port screen title (28), ship name (22), and zone name and tip title (19).
- **Body** (Fira Sans Regular, 14-18px): prose in the welcome poster (18), help pamphlet and banners (15-16), and hints in thin ink (14).
- **Action** (Fira Sans SemiBold, 17px): the verb on an action slip or button ("Abordar", "Zarpar com o guia").
- **Label** (Fira Sans SemiBold, 12-13px, uppercase): stat labels (CASCO, CARGA, BOMBORDO) and stamp text.
- **Keycap** (Fira Sans SemiBold, 15px): key glyphs.

### Named Rules
**The Wood-Type-Is-Rare Rule.** Alfa Slab One is used only for headlines, titles and the masthead. Running text, labels and numbers are always set in Fira Sans.

**The World-Text Rule (v63).** Text drawn in the world (plates, port and island names, signs, damage) is rasterized at on-screen size (`crisp_text`) and carries a dark drop shadow, so it stays sharp at any zoom and readable over sand and light water.

**The Translated-Key Rule.** Every string goes through `i18n::tr`, with the PT-BR text as the key and EN as its value. Layouts must survive EN strings being about 10-20% longer or shorter.

## Layout

The sea fills the viewport. Slips are absolutely positioned against a 16px margin: ship, hull and cargo at top-left; zone at top-center; wind and ammunition at top-right; objective at bottom-left; action and cannons at bottom-center; the F1 pamphlet slip at bottom-right. The action slip and the objective slip use fixed slots. The ship moves, so a slip that followed it would collide with the corner slips at 1366x768. A dashed ink leader line (gizmos, 14 on and 10 off, inset from hull and target) connects the ship to whatever the slip names.

Spacing inside slips runs 4 / 6 / 8 / 10 / 12px gaps. Posters open up to 14px row gaps and 26-36px padding. Modals (welcome, help, login) center on an ink scrim. On small screens the guide slip collapses to a compact bottom strip while docked. UI scale never drops below 0.85.

## Elevation & Depth

There is one elevation: paper resting on water. Every slip casts the same soft shadow, pushed downward and blurred (`0 3px 6px` in paper-shadow, no spread). Modals add an ink scrim beneath them, never a second shadow tier. Stacking uses `GlobalZIndex`: guide and tip slips at 20, the graduation headline at 30, the welcome modal at 45.

### Shadow Vocabulary
- **Paper lift** (`box-shadow: 0 3px 6px 0 rgba(5,13,23,0.45)`): every slip made by `ui::panel`.

### Named Rules
**The Paper-On-Water Rule.** Depth is always a soft offset blur, never a hard offset block, a glow or a halo.

## Shapes

Cut paper, not cards. Slips have a 2px ink border and 2px corners. Keycaps round to 3px and buttons to 4px, the softest shape in the system. Rules are woodcut: a single 1px ink rule, or a double rule of 3px and 1px separated by 2px under mastheads and slip headers. Bars are 8px tall with 3px ends.

## Components

### Slip (panel)
The unit of the world. Rag paper, 2px press-ink border, 2px corners, 8px by 12px padding, paper-lift shadow. Warning slips swap the border to vermilion. Fading slips (banner, PvP warning, toasts) fade in, hold, then fade out, and alpha is applied to background, border and text together.

### Masthead
A wood-type headline printed twice. A vermilion copy sits offset down and right by `size/16` px (clamped to 1.5-4px) beneath the ink copy, like a two-pass press out of register. It always sits over a double rule. It is the screen's typographic moment and appears on login, welcome, graduation and the help pamphlet.

### Buttons
- **Shape:** 4px corners, 1px ink border at 45% alpha.
- **Default:** aged paper, 5px by 10px padding, action type.
- **Primary:** brass wash, 8px by 14px padding, with its keycap printed at the left.
- **Hover / Pressed:** hover mixes the base 55% toward brass wash. Pressed fills with brass.

### Keycap
A printed key in a small box: aged paper, 1.5px ink border, 3px corners, 30px minimum width, bold 15px glyph. `input::glyph` swaps the label per device, from W/Q/Enter/Esc on keyboard to D-pad, LB/RB, A/X/Y and Start on gamepad. Wherever an action is named, its keycap is printed beside it.

### Stamp
A state mark: a 2px border and uppercase bold 13px text, both in the state color (teal for safe, ochre for frontier, vermilion ink for risk), with no fill. It is used for the zone tag and the FEITO step marker.

### Inputs / Fields
Aged paper at 60%, with only a 2px ink underline, 34px minimum height and bold 19px text. Focus turns the underline vermilion and appends a `|` caret. Tab cycles between fields.

### Bar
An 8px ink-tint track with a percentage fill (teal for hull and reload). Width is clamped to 0-100%.

### Leader line
A dashed press-ink line at 80% alpha, drawn with gizmos in world space from the hull to the target named by the action or guide slip.

## Do's and Don'ts

### Do:
- **Do** build every surface from `ui::panel`, `ui::masthead`, `ui::double_rule`, `ui::keycap`, `ui::stamp` and `ui::button`. Never restate their values inline.
- **Do** print the device-correct keycap next to every named action.
- **Do** anchor HUD slips to the 16px margin and keep the sea center clear.
- **Do** use the `-ink` color variants for any text under 20px on paper.
- **Do** route every string through `i18n::tr` with a PT-BR key and an EN value.

### Don't:
- **Don't** use dark translucent panels with metal borders. The ink scrim behind modals is the only dark surface.
- **Don't** set running text, labels or numbers in Alfa Slab One.
- **Don't** add a third press color or gradients. Vermilion stays reserved for danger, stamps and the masthead's second pass.
- **Don't** replace the soft paper-lift shadow with a hard offset, glow or halo.
- **Don't** put more than one masthead on a screen.

## Known Gaps

These were deferred at the finish review (2026-09-24). They are not rules to inherit.

- The deckled paper edge (procedural 9-slice torn border) is not built. Slips ship with clean 2px ink borders.
- Rotated ink stamps for zone state are not built. Stamps render upright.
- The port-screen title is plain 28px display type, without masthead misregistration.
- The guide slip still shows the eyebrow "PRIMEIRA VIAGEM · N DE 6" above the objective heading. This is a defect, not a label pattern: don't reuse it.
- The sea-event headline hides while a zone banner or PvP warning is up, but the zone banner and the PvP warning can still share a viewport on entering lawless water. This breaks the one-masthead rule.
