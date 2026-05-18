# Real-Session Validation

Validated on 2026-05-18 in a live Hyprland session with `us,th` layouts.

Results:

- Per-window restore worked with Firefox and VSCodium/Codium. Firefox restored layout `1` (`Thai`) and Codium restored layout `0` (`English (US)`) while switching focus back and forth.
- A new unsaved `hpwl-default-test` window defaulted to layout `0`.
- A configured class default for `hpwl-default-test` changed a new window to layout `1`.
- The single-instance guard rejected a second running copy with a clear already-running message.
- `hyprctl reload` did not break layout restore behavior in this session.
- During quick focus switching, `activewindow` consistently arrived before the matching `activewindowv2`.

Caveats:

- Excluded-keyboard behavior was not fully proven in the real session because no excluded keyboard emitted an `activelayout` event.
- Configured include mode worked for the included keyboard, but Hyprland/device grouping may still mirror layout changes across related devices.
