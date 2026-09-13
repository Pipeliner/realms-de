# A feature-gated realm-control client installed only in checks.session-boots.
# It gives the Python VM driver a typed production Client without adding a
# user-facing realmctl command or duplicating the wire protocol in the test.
{
  pkgs,
  lib,
  support,
  src,
}:
(support.rustPlatformFor pkgs).buildRustPackage {
  pname = "realm-vm-control";
  inherit (support) version;

  src = lib.cleanSourceWith {
    inherit src;
    filter =
      path: _type:
      !(builtins.elem (baseNameOf path) [
        "target"
        "result"
        ".direnv"
      ]);
  };

  cargoLock.lockFile = src + "/Cargo.lock";
  cargoBuildFlags = [
    "--package"
    "realm-control"
    "--features"
    "vm-test-helper"
    "--bin"
    "realm-vm-control"
  ];
  doCheck = false;

  installPhase = ''
    runHook preInstall
    install -Dm755 \
      target/${pkgs.stdenv.targetPlatform.rust.cargoShortTarget}/release/realm-vm-control \
      $out/bin/realm-vm-control
    runHook postInstall
  '';
}
