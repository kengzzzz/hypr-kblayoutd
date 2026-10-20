# Changelog

## 0.4.0 - 2026-10-20

Fewer devices treated as keyboards, warnings for config mistakes, and the AUR packages kept in this repo.

- Keyboards: power buttons, sleep buttons, `video-bus` and media-key devices (`consumer-control`, `system-control`) are no longer learned. They never type, but any `switchxkblayout all` made the daemon learn them and switch them along with real keyboards. Setting `exclude_contains` still replaces the defaults.
- Config: unknown keys, like a misspelled `[layer_layout]`, are logged as warnings instead of being ignored.
- Config: a layout index past the end of `input:kb_layout` is logged at startup.
- Dependencies: all updated, including `toml` 1.x. The binary is slightly smaller.
- Packaging: the `-bin` and `-git` AUR PKGBUILDs live in `packaging/`, and both install this changelog.
- Releases: notes come from this file instead of PR titles.

## 0.3.1 - 2026-08-10

- Keyboards: a learned keyboard that disappears, such as an unplugged one, is forgotten instead of failing every switch.

## 0.3.0 - 2026-08-04

Layouts for launchers, and Nix support.

- Launchers: `[layer_layouts]` gives layer surfaces such as rofi their own remembered layout, separate from the window underneath.
- Nix: a flake with a package, an overlay and a Home Manager module. Thanks to @assembledev.

## 0.2.0 - 2026-07-21

- Config: `default_layouts` keys can be regexes, matched against the whole class.
- A systemd user unit for uwsm sessions, in `contrib/`.
- Fixed stale state, the daemon's own switches being taken for manual ones, and failed switch handling.
- Release tarballs for x86_64 and aarch64.
