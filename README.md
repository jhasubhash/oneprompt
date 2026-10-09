# oneprompt

A GPU terminal (Rust + wgpu) where the surface and the type are part of the design.
No more black rectangle with a generic monospace font: themes pick a texture,
an ink colour and a typeface, for example *ink on paper* or *chalk on slate*.

| Ink on Paper | Chalkboard | Classic |
|---|---|---|
| ![](docs/screenshots/ink-on-paper.png) | ![](docs/screenshots/chalkboard.png) | ![](docs/screenshots/classic.png) |

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
| Cmd+T | next theme |
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
tabs/splits, a config file.

## Layout

```
src/main.rs                 window events, shortcuts, wiring
src/terminal.rs             PTY + alacritty_terminal grid, resize, scroll, paste
src/input.rs                key presses -> xterm byte sequences
src/boxdraw.rs              box-drawing and block characters as rectangles
src/renderer.rs             wgpu passes: background, cell rectangles, text; PNG snapshots
src/theme.rs                themes as plain data
src/shaders/background.wgsl procedural textures
src/shaders/rect.wgsl       cell backgrounds, cursor, underlines
```

## Roadmap

1. **Proportional and handwritten fonts.** The grid needs a fixed advance, so themes
   use monospace faces today (Courier New, PT Mono, Menlo). Faces like American
   Typewriter or Chalkboard need per-cell glyph placement (centre each glyph in its cell).
2. **Input and navigation.** IME, clickable links, search in scrollback, tabs.
3. **Ink shader.** Render text to an offscreen texture, then composite it with the paper:
   multiply blending, slightly darker glyph edges, and noise-driven pressure and bleed
   so letters look absorbed rather than printed on top.
4. **Theme files.** TOML themes with hot reload, image textures (scanned paper, linen)
   alongside procedural ones, and per-role fonts (prompt, output, bold, italic).
5. **Polish.** Cursor styles (nib, chalk stub), selection as highlighter, ligatures, IME.
