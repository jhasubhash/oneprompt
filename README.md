# oneprompt

A GPU terminal (Rust + wgpu) where the surface and the type are part of the design.
No more black rectangle with a generic monospace font: the theme picks a texture,
an ink colour and a typeface.

| Tsushima Light | Tsushima Dark |
|---|---|
| ![](docs/screenshots/tsushima-light.png) | ![](docs/screenshots/tsushima-dark.png) |

## Theme

A theme is the whole look: surface texture, ink behaviour, palette, font and the
window controls. There is one for now, **Tsushima**, after the ink-wash look of
*Ghost of Tsushima*, in two variants:

- **Light**: rough washi with relief-lit tooth, hard-edged water-stain tide lines, fibers
  and a flick of crimson splatter; bold sumi ink swept with a slanted brush nib
  (thick-and-thin strokes, dry-brush streaks, varying pressure) whose ragged edges bleed in
  patches along the fibers; a crimson seal for the cursor and close control.
- **Dark**: after *konshi*, the indigo-black dyed paper sutras were written on in gold and
  white pigment: the same brushwork in pale gofun ink, stains that dried lighter than the
  sheet, the same crimson seal.

The variant follows macOS light / dark mode and switches with it; Cmd+T flips it for the
session, and `appearance` in the settings pins one. Every session gets a new sheet of paper
(`ONEPROMPT_SEED=<number>` pins one). The renderer still supports the earlier surfaces
(cotton paper, chalkboard, flat) and control styles for themes to come.

Rendering ink is a two-step process: text, line art and controls are drawn into an
offscreen *ink layer*, then `src/shaders/ink.wgsl` composites it onto the surface with
a brush nib (glyphs swept along a slanted nib for thick-and-thin strokes), bleed (ink
wicking along paper fibers), dry-brush grain and pooled rims. Each theme sets those
amounts and its font weight in `theme.rs`; all zero gives crisp screen text.

On macOS the Metal layer is explicitly tagged sRGB. Left untagged, wide-gamut displays
colour-match some frames and not others, which shows up as the window flickering in tint
while you type.

## Status

Milestone 1, a working terminal:

- Your login shell in a PTY, parsed by `alacritty_terminal` (VT sequences, scrollback, alternate screen)
- Grid rendering with glyphon / cosmic-text, one monospace font per theme
- ANSI colours mapped to per-theme palettes ("inks" on paper, "chalks" on slate); 256-colour and true colour pass through
- Bold, italic, underline, strikeout, inverse, dim; block, beam and underline cursors (outlined when unfocused)
- Mouse selection drawn as a per-theme highlighter; double-click selects a word, triple-click a line
- Mouse reporting for full-screen apps (vim, htop, tmux): clicks, drags, wheel, SGR and legacy encodings
- Box-drawing and block characters drawn as shapes, so borders and bars join without seams
- Focus reporting, left Option as Meta (right Option still types é, €, ...)
- Procedural backgrounds in WGSL (`src/shaders/background.wgsl`): paper fibers and grain, chalkboard smudges and dust, flat

| Key | Action |
|---|---|
| Cmd+, | open settings |
| Cmd+T | flip between light and dark |
| Cmd+W / Cmd+M | close / minimize (or the theme's controls, top left) |
| drag top strip | move the window; double-click it to zoom |
| Cmd+= / Cmd+- | bigger / smaller text |
| Cmd+C | copy selection |
| Cmd+V | paste (bracketed when the app asks for it) |
| drag / double / triple click | select text / word / line |
| Shift+drag | select even when an app has taken the mouse |
| left Option+key | Meta (Alt+B, Alt+F, Alt+. ...); Option+←/→ moves by word |
| scroll | scrollback, arrow keys in less/vim, or wheel events if the app wants the mouse |

```sh
cargo run                                       # your login shell
cargo run -- -e "htop"                          # run a command via /bin/sh instead
ONEPROMPT_SNAPSHOT=docs/screenshots cargo run -- -e "..."   # render every theme to PNG and exit
```

Not yet: IME (composed input for Chinese, Japanese, ...), clickable links, search,
tabs/splits.

## Settings

Cmd+, opens `~/.config/oneprompt/config.toml` in your default text editor, creating it
with every option documented and commented out. Saving applies changes immediately; a
note in the title strip confirms it, or says what is wrong and keeps the previous
settings.

```toml
appearance = "system"         # system | light | dark
option_as_meta = "left"       # left | both | none
seed = 11                     # pin the paper; unset = new sheet each session

[themes.tsushima-dark]        # each variant: fonts, line_height, bleed, nib, grain, edge
font_family = "Courier New"
font_weight = 700
nib = 1.2
```

The file starts with both variants' built-in values as active lines, so editing a number
just works (`oneprompt --default-config` prints it). Top-level `font_family`, `font_size`
and `font_weight` also exist and apply to any variant whose section doesn't set them.
Font weights snap to the faces the font has (Courier New: 400 and 700); the text engine
would otherwise fall back to a different, proportional font. A font that is missing or not
monospace gets a warning in the title strip. `ONEPROMPT_CONFIG=<path>` uses a different file.

## Layout

```
src/main.rs                 window events, shortcuts, wiring
src/terminal.rs             PTY + alacritty_terminal grid, resize, scroll, paste
src/input.rs                key presses -> xterm byte sequences
src/boxdraw.rs              box-drawing and block characters as rectangles
src/chrome.rs               theme-drawn window controls: layout and hit testing
src/config.rs               settings file: parsing, theme overrides, live reload
src/renderer.rs             ink pass (text, line art, controls) + main pass (surface, cells, ink composite)
src/theme.rs                themes as plain data
src/shaders/background.wgsl procedural textures
src/shaders/rect.wgsl       rectangles, brush discs, rings and crosses
src/shaders/ink.wgsl        ink-on-surface composite: bleed, grain, pooled rims
```

## Roadmap

1. **Proportional and handwritten fonts.** The grid needs a fixed advance, so themes
   use monospace faces today (Courier New, PT Mono, Menlo). Faces like American
   Typewriter or Chalkboard need per-cell glyph placement (centre each glyph in its cell).
2. **Input and navigation.** IME, clickable links, search in scrollback, tabs.
3. **Brush typography.** Stroke-width variation (pressure) and per-glyph wobble, a
   brush-style monospace face for Tsushima.
4. **Theme files.** TOML themes with hot reload, image textures (scanned paper, linen)
   alongside procedural ones, and per-role fonts (prompt, output, bold, italic).
5. **Polish.** Cursor styles (nib, chalk stub), selection as highlighter, ligatures, IME.
