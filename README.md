# hypr-kblayoutd

Small Hyprland daemon that remembers keyboard layout per window and restores the saved layout when focus changes.

## Behavior

- Listens to Hyprland's event socket; it does not poll.
- Remembers the active layout for each window address.
- Restores a remembered layout when that window becomes active again.
- Gives new windows layout `0`, unless a class default is configured.
- Optionally remembers a layout for layer-shell launchers (rofi, wofi, …), separately from the window underneath.
- Seeds the active keyboard at startup and learns additional keyboards from `activelayout` events unless keyboards are explicitly configured.
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

# Keys are also regexes (matched against the whole class), like Hyprland
# window rules. Exact entries win over patterns. Handy for Chrome PWAs:
"chrome-.*whatsapp.*" = 1

# Layer-shell surfaces that take the keyboard while they are open, keyed by
# namespace. Empty by default; listing one opts it into layout tracking.
[layer_layouts]
"rofi" = 0
```

Layout indexes follow Hyprland's `input:kb_layout` order. For example, `kb_layout = us,th` means `us` is `0` and `th` is `1`.

Useful discovery commands:

```sh
hyprctl devices -j
hyprctl clients
hyprctl layers   # namespaces for [layer_layouts]
```

### Launchers and other layer surfaces

Wayland launchers are not windows: they are layer-shell surfaces that take
keyboard focus without changing Hyprland's active window. Without
`[layer_layouts]` they inherit whatever the window underneath is using, and a
layout switched while the launcher is open is wrongly remembered for that
window.

Add a namespace to `[layer_layouts]` to give it its own layout:

- Opening it applies its remembered layout, or the configured value the first
  time round.
- A layout change while it is open belongs to the launcher, not the window.
- Closing it restores the window underneath (or an outer layer, if stacked).
- Keys are regexes here too, matched against the whole namespace.

Only namespaces you list are tracked, because Hyprland's `openlayer` event
fires for every layer surface — including bars and wallpapers, which never take
the keyboard. These IPC events report mapping and unmapping, not keyboard focus,
so only opt in a surface whose mapped lifetime matches the time it owns keyboard
focus. Persistent layers or surfaces that change keyboard interactivity while
remaining mapped cannot be tracked reliably through Hyprland's IPC events.

## Installation

### Nix

Run the daemon directly:

```sh
nix run github:kengzzzz/hypr-kblayoutd
```

The flake also exports `packages`, `apps`, an overlay, and a Home Manager
module. In a NixOS configuration using Home Manager, pass the flake input to
the module and add:

```nix
{
  home-manager.sharedModules = [ hypr-kblayoutd.homeManagerModules.default ];

  home-manager.users.your-user = {
    services.hypr-kblayoutd = {
      enable = true;
      settings = {
        keyboards.exclude_contains = [
          "wlr_virtual_keyboard_v"
          "yubikey"
        ];
        default_layouts.firefox = 0;
      };
    };
  };
}
```

When `settings` is non-empty, the module writes `config.toml`; otherwise it
leaves that path unmanaged. It manages the daemon as a systemd user service
attached to `graphical-session.target`.

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

### systemd user unit (uwsm)

If you run Hyprland under uwsm, use the shipped user unit instead of `exec-once`:

```sh
systemctl --user enable --now hypr-kblayoutd.service
```

The AUR package installs the unit. When building from source, install it manually first (adjust `ExecStart` if the binary is not in `/usr/bin`):

```sh
install -Dm644 contrib/hypr-kblayoutd.service ~/.config/systemd/user/hypr-kblayoutd.service
```

Hyprland must have at least two keyboard layouts configured, unless you use `input:kb_file`.

## Logging

Normal operation is quiet. Enable debug logs with:

```sh
RUST_LOG=debug hypr-kblayoutd
```
