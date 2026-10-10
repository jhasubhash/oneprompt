# Working on oneprompt

oneprompt is a macOS terminal (Rust, wgpu, winit, glyphon, alacritty_terminal) whose point
is how it looks: a hand-made paper sheet, ink that bleeds, text written in by hand. Visual
quality comes before terminal feature breadth. README.md is the short
pitch; docs/GUIDE.md describes every feature, setting and the source layout.

## Build and check

```sh
scripts/build.sh               # cargo build --release, signed so macOS keeps the app's permissions
cargo test --release
cargo clippy --release         # keep it warning-free
```

## After every change

1. Build, test and lint as above.
2. Look at the result, in **Tsushima Light** (or the light variant of the theme you
   changed), unless the change only affects dark mode.
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
   it never needs remaking after a build. Build with `scripts/build.sh`, not a bare
   `cargo build --release`: it signs the binary with the Developer ID certificate and a fixed
   identifier, and macOS keys the permissions programs ask for in the terminal (Photos,
   folders) on that signature; an ad-hoc build is a new app to it, and every permission is
   asked for again.

4. **Keep the docs current**: update docs/GUIDE.md whenever the code changes make it stale
   (features, settings, source layout, build steps), in the same change. Keep README.md
   short: it is the pitch; touch it only for headline features or install changes.

## Testing in the GUI without disturbing the user

- Capture only the test window (`screencapture -l <window id>`), never the whole screen.
  Video (`screencapture -v -R`) records a region, so it also catches whatever is beneath
  once the test window moves or closes: use it only where nothing of the user's is
  behind.
- The user's own oneprompt is usually running too: pick the test window by its process's
  command line (`ps -o pid,command -p <pid>`), and never send keys to or resize the user's.
- Start test windows with `ONEPROMPT_BACKGROUND=1`: they open behind whatever is in front,
  so they never take the keyboard from the user (a test window coming to the front has
  caught the user's typing).
- Never send keystrokes (System Events) while the user's oneprompt is running: macOS can't
  tell the two apart (same executable), and keys sent to the test window have landed in the
  user's. Script the test shell instead: start it with `ZDOTDIR` pointing at a folder whose
  `.zshrc` feeds queued commands from a `zle-line-init` widget (shell integration restores
  that `ZDOTDIR` for zsh), so it types its own commands. For the app's own actions,
  `ONEPROMPT_ACTIONS="2:clear,4.5:new-tab,6:close-tab"` plays menu actions (by their names
  in kebab case) at those seconds after start.
- No synthetic mouse clicks: window coordinates do not map reliably across the user's
  displays, and clicks have landed in other apps. Ask the user to try mouse behaviour.
- Tests that use Cmd+C / Cmd+V save the clipboard first and restore it afterwards.

## Code

- Match the surrounding code: comments explain why, in plain sentences; names are spelled
  out; British spelling in prose ("colour").
- Colours are authored in sRGB; shaders work on those values directly (the Metal layer is
  tagged sRGB). Texture and ink sizes are in logical pixels, scaled by the window's scale.
- Look and feel belongs in a theme (`src/themes/<name>/`), never in the framework. Themes
  use only `crate::theme` (the API) and never each other (`themes::tests` checks it); what
  a theme needs that the API lacks goes into the API. CONTRIBUTING.md says how to add one.
- Theme settings are declared by the theme (`Theme::params`); the settings template,
  validation and settings panel rows are generated from them.
- New settings: validate their range, apply them live, add them to the template and a row
  in the settings panel (`settings::items`), document them in docs/GUIDE.md.

## Releases

Only when the user asks for one. Releases are alphas for now, published as GitHub
pre-releases.

1. Bump `version` in Cargo.toml (`0.1.0-alpha.N`) and build, so Cargo.lock follows.
2. `scripts/release.sh`: builds the universal app, signs it, makes
   `target/dist/oneprompt-<version>.dmg` and notarizes it (several minutes). The signing
   certificate and the `oneprompt-notary` credentials are in the user's keychain; if they
   are missing, the user sets them up (the script's header says how): never type their
   Apple ID or passwords.
3. Commit the bump ("Release <version>") and push.
4. `gh release create v<version> <dmg> --prerelease --title "oneprompt <version>"` with
   notes: how to install, what is new since the last release, the dmg's SHA-256.
5. `scripts/update-cask.sh`: points the Homebrew cask (github.com/jhasubhash/homebrew-tap,
   `Casks/oneprompt.rb`) at the published dmg. Check it with
   `brew style` and `brew fetch --cask jhasubhash/tap/oneprompt` (the audit's "GitHub
   pre-release" complaint is expected). Don't install the cask on the user's machine: its
   `/Applications/oneprompt.app` would sit beside their dev bundle.

## Git

- Commit or push only when the user asks. Commit messages: a short imperative subject,
  then a body saying what changed and why.
- The remote is `origin` (github.com/jhasubhash/oneprompt), branch `main`.
