# Compile the existing private realm-theme unit-test harness for execution in a
# NixOS VM.  This is deliberately not a product binary or test API: the VM
# invokes one ignored unit-test selector as an unprivileged user against a
# root-owned ELF from the Nix store.
{
  pkgs,
  support,
  src,
}:
(support.rustPlatformFor pkgs).buildRustPackage {
  pname = "realm-theme-desktop-admission-vm-test";
  inherit (support) version;

  src = pkgs.lib.cleanSourceWith {
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
  doCheck = false;

  buildPhase = ''
    runHook preBuild
    cargo test --package realm-theme --lib --no-run --locked
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mapfile -t binaries < <(
      find target -type f -path '*/debug/deps/realm_theme-*' -perm -0100 -print
    )
    test "''${#binaries[@]}" -eq 1
    install -Dm755 "''${binaries[0]}" "$out/libexec/realm-theme-desktop-exec-tests"
    runHook postInstall
  '';
}
