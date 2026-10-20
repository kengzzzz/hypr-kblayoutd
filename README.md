# hypr-kblayoutd

Small Hyprland daemon that remembers keyboard layout per window.

## Behavior

- Listens to Hyprland's event socket, no polling.
- Remembers the layout of each window and restores it on focus.
- New windows get layout `0`, or their class default.
- Optionally gives layer-shell launchers (rofi, wofi, …) their own layout.
- Learns keyboards from `activelayout` events, unless `include` lists them.
- Switches layouts over Hyprland's command socket, not by spawning `hyprctl`.
- Reconnects with backoff if the event socket drops.

## Configuration

Optional. Read from `~/.config/hypr-kblayoutd/config.toml`:

```toml
[keyboards]
# Only switch and watch these keyboards. Empty (default): learn them as they are used.
include = ["keychron-keychron-k2"]

# Ignore learned keyboards whose name contains any of these. Unused with `include`.
# Setting it replaces the defaults: wlr_virtual_keyboard_v, yubikey, power-button,
# sleep-button, video-bus, consumer-control, system-control.
exclude_contains = ["wlr_virtual_keyboard_v", "yubikey"]

[default_layouts]
"org.telegram.desktop" = 1
"firefox" = 0
# Keys are also regexes matched against the whole class. Exact keys win.
"chrome-.*whatsapp.*" = 1

# Layer surfaces to track, by namespace. Empty by default.
[layer_layouts]
"rofi" = 0
```

Layout indexes follow `input:kb_layout`: with `us,th`, `us` is `0` and `th` is `1`.

Find names with:

```sh
hyprctl devices -j   # keyboards
hyprctl clients      # classes
hyprctl layers       # namespaces
```

### Launchers and other layer surfaces

Launchers are layer surfaces, not windows: they take the keyboard without changing the active window. Untracked, they inherit the window's layout, and a switch made in them is saved to that window.

A namespace in `[layer_layouts]`:

- Gets its remembered layout on open, or the configured one the first time.
- Keeps layout changes made while it is open.
- Restores the window (or the outer layer) on close.
- May be a regex matched against the whole namespace.

Only list surfaces that hold the keyboard for as long as they are mapped. Hyprland reports layer map and unmap, not keyboard focus, and `openlayer` fires for bars and wallpapers too. Surfaces that stay mapped while toggling keyboard focus can't be tracked.

`j/layers` also lists surfaces playing their close animation. Events handle this, but a startup or reconnect snapshot can't tell them apart from open ones. A layer closing at that moment may be kept until the next layer event or reconnect.

## Installation

### Nix

```sh
nix run github:kengzzzz/hypr-kblayoutd
```

The flake also exports `packages`, `apps`, an overlay and a Home Manager module:

```nix
{
  home-manager.sharedModules = [ hypr-kblayoutd.homeManagerModules.default ];

  home-manager.users.your-user = {
    services.hypr-kblayoutd = {
      enable = true;
      settings = {
        default_layouts.firefox = 0;
        layer_layouts.rofi = 0;
      };
    };
  };
}
```

Non-empty `settings` are written to `config.toml`. Empty `settings` leave it unmanaged. The daemon runs as a user service under `graphical-session.target`.

### AUR

```sh
paru -S hypr-kblayoutd-bin   # release binary
paru -S hypr-kblayoutd-git   # latest main, built from source
```

PKGBUILDs: [`packaging/`](packaging/README.md).

### Build from source

```sh
cargo build --release
```

Start it from `hyprland.conf`:

```text
exec-once = hypr-kblayoutd
```

### systemd user unit (uwsm)

Under uwsm, use the user unit instead of `exec-once`:

```sh
systemctl --user enable --now hypr-kblayoutd.service
```

The AUR packages install it. From source, install it first (and fix `ExecStart` if the binary is not in `/usr/bin`):

```sh
install -Dm644 contrib/hypr-kblayoutd.service ~/.config/systemd/user/hypr-kblayoutd.service
```

Hyprland needs at least two layouts in `input:kb_layout`, unless `input:kb_file` is set.

## Logging

Quiet by default. For debug logs:

```sh
RUST_LOG=debug hypr-kblayoutd
```
