{
  lib,
  rustPlatform,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  inherit (cargoToml.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.lock
      ../Cargo.toml
      ../src
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  meta = {
    inherit (cargoToml.package) description;
    homepage = cargoToml.package.repository;
    license = lib.licenses.mit;
    mainProgram = "hypr-kblayoutd";
    platforms = lib.platforms.linux;
  };
}
