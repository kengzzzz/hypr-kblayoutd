{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.hypr-kblayoutd;
  toml = pkgs.formats.toml { };
  configFile = toml.generate "hypr-kblayoutd-config.toml" cfg.settings;
in
{
  options.services.hypr-kblayoutd = {
    enable = lib.mkEnableOption "per-window keyboard layouts in Hyprland";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The hypr-kblayoutd package to run.";
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = {
        keyboards.exclude_contains = [
          "wlr_virtual_keyboard_v"
          "yubikey"
        ];
        default_layouts.firefox = 0;
      };
      description = ''
        Settings written to
        {file}`$XDG_CONFIG_HOME/hypr-kblayoutd/config.toml`. When empty,
        Home Manager leaves that path unmanaged.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    xdg.configFile = lib.mkIf (cfg.settings != { }) {
      "hypr-kblayoutd/config.toml".source = configFile;
    };

    systemd.user.services.hypr-kblayoutd = {
      Unit = {
        Description = "Per-window keyboard layout daemon for Hyprland";
        Documentation = "https://github.com/kengzzzz/hypr-kblayoutd";
        After = [ "graphical-session.target" ];
        PartOf = [ "graphical-session.target" ];
        X-Restart-Triggers = [
          cfg.package
          configFile
        ];
      };

      Service = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
        RestartSec = 1;
        Slice = "background-graphical.slice";
      };

      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
