{
  description = "Per-window keyboard layout daemon for Hyprland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      overlays.default = final: _prev: {
        hypr-kblayoutd = final.callPackage ./nix/package.nix { };
      };

      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ self.overlays.default ];
          };
        in
        {
          inherit (pkgs) hypr-kblayoutd;
          default = pkgs.hypr-kblayoutd;
        }
      );

      apps = forAllSystems (
        system:
        let
          app = {
            type = "app";
            program = nixpkgs.lib.getExe self.packages.${system}.default;
            meta.description = "Run hypr-kblayoutd";
          };
        in
        {
          default = app;
          hypr-kblayoutd = app;
        }
      );

      checks = forAllSystems (system: {
        package = self.packages.${system}.default;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              rustc
              rustfmt
            ];
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt-tree);

      homeManagerModules.default =
        { lib, pkgs, ... }:
        {
          imports = [ ./nix/home-manager.nix ];
          services.hypr-kblayoutd.package =
            lib.mkDefault
              self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
    };
}
