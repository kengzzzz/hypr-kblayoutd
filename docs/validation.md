# Real-Session Validation

Validated 2026-05-18 in a live Hyprland session with `us,th`.

Results:

- Per-window restore worked switching between Firefox (layout `1`, Thai) and Codium (layout `0`, English (US)).
- A new `hpwl-default-test` window got layout `0`, or layout `1` with a class default.
- A second instance was rejected with an already-running message.
- `hyprctl reload` did not break restoring.
- Under fast focus switching, `activewindow` always arrived before its `activewindowv2`.

Caveats:

- Exclusion was not proven: no excluded keyboard emitted `activelayout`.
- `include` worked, but Hyprland may still mirror layout changes across related devices.
