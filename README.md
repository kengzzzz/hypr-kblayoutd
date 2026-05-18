# hypr-kblayoutd

Small Hyprland daemon that remembers keyboard layout per window and restores the saved layout when focus changes.

## Behavior

- Listens to Hyprland's event socket; it does not poll.
- Remembers the active layout for each window address.
- Restores a remembered layout when that window becomes active again.
- Gives new windows layout `0`, unless a class default is configured.
- Learns keyboards from `activelayout` events unless keyboards are explicitly configured.
- Switches layouts through Hyprland's command socket directly, avoiding a `hyprctl` process per keyboard.
- Reconnects to the Hyprland event socket with backoff if the socket disconnects.

## Configuration

Config is optional. If present, it is read from:

```text
~/.config/hypr-kblayoutd/config.toml
```

Example:

```toml
[keyboards]
# If non-empty, only these keyboards are switched and watched.
include = ["keychron-keychron-k2"]

# Used for learned keyboards. Defaults to ["wlr_virtual_keyboard_v", "yubikey"].
exclude_contains = ["wlr_virtual_keyboard_v", "yubikey"]

[default_layouts]
"org.telegram.desktop" = 1
"discord" = 1
"firefox" = 0
```

Layout indexes follow Hyprland's `input:kb_layout` order. For example, `kb_layout = us,th` means `us` is `0` and `th` is `1`.

Useful discovery commands:

```sh
hyprctl devices -j
hyprctl clients
```

## Installation

### AUR

```sh
paru -S hypr-kblayoutd-git
# or
yay -S hypr-kblayoutd-git
```

### Build from source

```sh
cargo build --release
```

Run from `hyprland.conf`:

```text
exec-once = hypr-kblayoutd
```

Hyprland must have at least two keyboard layouts configured, unless you use `input:kb_file`.

## Logging

Normal operation is quiet. Enable debug logs with:

```sh
RUST_LOG=debug hypr-kblayoutd
```
