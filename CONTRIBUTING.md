# Contributing to oneprompt

Thanks for your interest. oneprompt is a macOS terminal whose point is how it looks, so
visual quality comes before terminal feature breadth. Please keep that in mind when
proposing changes. README.md describes the features and the source layout.

## Before you start

For anything bigger than a small fix, open an issue first so we can agree on the
approach before you spend time on it. Security problems should not go in a public issue;
see [SECURITY.md](SECURITY.md).

## Build and check

Always build in release mode; debug builds are not used.

```sh
cargo build --release
cargo test --release
cargo clippy --release         # keep it warning-free
```

## Look at your change

Run the result and look at it, in the light variant (Tsushima Light, or the light variant
of the theme you changed) unless the change only affects dark mode. A separate settings file keeps your own untouched:

```sh
printf 'appearance = "light"\nseed = 11\n' > /tmp/op-test.toml
ONEPROMPT_CONFIG=/tmp/op-test.toml ./target/release/oneprompt -e "ls; sleep 30"
```

To render still frames without a window, `ONEPROMPT_SNAPSHOT=<dir>` writes one PNG per
theme and exits; `ONEPROMPT_SEED=<n>` pins the paper pattern. For visual changes, please
include a screenshot or short recording in your pull request.

## Code style

- Match the surrounding code: comments explain why, in plain sentences; names are spelled
  out; British spelling in prose ("colour").
- Colours are authored in sRGB, and shaders work on those values directly. Texture and ink
  sizes are in logical pixels, scaled by the window's scale.
- Look and feel belongs in a theme (`src/themes/<name>/`), not in the framework. A theme
  only uses the theme API (`crate::theme`), never another theme or the renderer; a test
  enforces it. If a theme needs something the API lacks, add it to the API so every
  theme can use it.
- A new setting needs a validated range, must apply live, must be added to the generated
  template, and must be documented in README.md.
- Update README.md whenever your change makes it stale.

## Adding a theme

1. Make a folder `src/themes/<name>/` with a `mod.rs` implementing `theme::Theme` (see
   `src/theme/mod.rs` for the API, and Tsushima or Chalkboard as examples): its id, name,
   one sentence about it, a light and a dark variant, and the settings it declares.
2. Write the stages it draws itself, as WGSL next to it: `surface`, `theme_ink`,
   `theme_shape`, `theme_transition`. Leave out what it doesn't need; the framework's
   plain defaults fill in. Values come from `Theme::data` (and `Plan` for transitions)
   into a struct the theme declares.
3. Give it its reactions: `reveal`, `switch`, `cursor`, `strike` / `draw_strike`, and
   `draw_divider` for the line between split panes (drawn in when a split is made).
4. Add it to `all()` in `src/themes/mod.rs`. It then appears in the picker, the
   settings panel (a slider per setting it declares) and the settings template, and
   `cargo test` compiles its shaders.
5. Look at both variants (`ONEPROMPT_SNAPSHOT=<dir>` renders every variant;
   `ONEPROMPT_SNAPSHOT_SWITCH=<n>` adds frames of each switch), add screenshots to
   `docs/screenshots/` and a short description to README.md.

## Pull requests

- Keep each pull request to one change.
- Commit messages: a short imperative subject, then a body saying what changed and why.
- Make sure build, tests and clippy pass before you open it.

## Licence

oneprompt is licensed under the [Apache License, Version 2.0](LICENSE). By submitting a
contribution you agree that it is licensed under the same terms. Keep the copyright
notices and the [NOTICE](NOTICE) file intact.
