# Working on oneprompt

oneprompt is a macOS terminal (Rust, wgpu, winit, glyphon, alacritty_terminal) whose point
is how it looks: a hand-made paper sheet, ink that bleeds, text written in by hand. Visual
quality comes before terminal feature breadth. README.md describes the features and the
source layout.

## Build and check

```sh
cargo build --release          # the build everything runs; always release
cargo test --release
cargo clippy --release         # keep it warning-free
```

## After every change

1. Build, test and lint as above.
2. Look at the result, in **Tsushima Light**, unless the change only affects dark mode.
   Test windows get their own settings file, so the user's is never touched:
   ```sh
   printf 'appearance = "light"\nseed = 11\n' > /tmp/op-test.toml
   ONEPROMPT_CONFIG=/tmp/op-test.toml ./target/release/oneprompt -e "ls; sleep 30"
   ```
   Render still frames without a window: `ONEPROMPT_SNAPSHOT=<dir>` writes one PNG per
   theme and exits; `ONEPROMPT_SEED=<n>` pins the paper pattern.
3. **Relaunch the user's app**: quit every running oneprompt and reopen the bundle, so the
   user is never left on an old build:
   ```sh
   pkill -x oneprompt; open ~/Applications/oneprompt.app
   ```
   The bundle's executable links to `target/release/oneprompt` (`scripts/make-app.sh`), so
   it never needs remaking after a build.

## Testing in the GUI without disturbing the user

- Capture only the test window (`screencapture -l <window id>`), never the whole screen.
- Before sending keystrokes (System Events), make the test window frontmost and check it
  is, or the keys go to whatever the user is using.
- No synthetic mouse clicks: window coordinates do not map reliably across the user's
  displays, and clicks have landed in other apps. Ask the user to try mouse behaviour.
- Tests that use Cmd+C / Cmd+V save the clipboard first and restore it afterwards.

## Code

- Match the surrounding code: comments explain why, in plain sentences; names are spelled
  out; British spelling in prose ("colour").
- Colours are authored in sRGB; shaders work on those values directly (the Metal layer is
  tagged sRGB). Texture and ink sizes are in logical pixels, scaled by the window's scale.
- Look and feel belongs in the theme (`src/theme.rs`) and settings (`src/config.rs`, whose
  template is generated from the themes), not in constants scattered through the renderer.
- New settings: validate their range, apply them live, add them to the template, document
  them in README.md.

## Git

- Commit or push only when the user asks. Commit messages: a short imperative subject,
  then a body saying what changed and why.
- The remote is `origin` (github.com/jhasubhash/oneprompt), branch `main`.
