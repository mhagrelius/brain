# Departures from the design handoff

Recorded departures — places where the app deliberately differs from
`docs/handoff/` (mock + README). The handoff is not runnable UI, so some
things it is silent on need a decision anyway.

## Modal & popover depth cues

**Veil behind capture/insert/search panels — kept, tokenized.**
The mock floats every panel with no scrim behind it (its grey page
background does the dimming). The app opens panels over live note
content, which needs a real dimmer: `Main.qml` has always drawn a
hardcoded `Qt.rgba(0,0,0,0.18)` veil over the workspace. Kept the
behaviour (an un-dimmed panel over text is hard to read), removed the
hardcode: the colour now lives in the palette as `T.veil`
(`rgba(0,0,0,.18)`, fixed — not theme-derived). Not in the handoff by
design of the mock; if the mock gains a scrim, retune the constant there.

**Shadows via MultiEffect, not literal CSS.**
The handoff speaks CSS (`box-shadow: 0 18px 40px rgba(0,0,0,.5)`).
QML has no box-shadow, so `components/DropShadow.qml` maps each value
literally onto `MultiEffect` (shadowVerticalOffset = the y-offset,
blurMax = the blur radius, shadowColor/Opacity from the rgba). One
component, two presets — `menu: true` for the search/menu class
(`0 22px 50px .55`), default for the popover class (`0 18px 40px .5`).
Colour is the fixed palette role `T.shadow` (black) so themes cannot
tint the shadow into a glow.

**RowMenu previously had no shadow at all** (flat panel over text);
now uses the popover preset like every other popover.

## Out of scope

- **Tooltips** — not specified in the handoff; left as-is.
- **FirstRun dialog** — native platform dialog, not a themed surface.
