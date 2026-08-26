# Shared facts for helm's Nix build: which toolchain, which river, which tools.
#
# Everything here is a decision with a reason attached. Imported by the root
# flake.nix and by both modules, so a pin is written down once.
{ lib, src }:
rec {
  version = "0.1.0";

  # ── toolchain ─────────────────────────────────────────────────────────────
  # Honour rust-toolchain.toml rather than assuming. `channel = "stable"` maps
  # onto nixpkgs' default rustPlatform; anything else needs a real toolchain
  # input and should say so loudly instead of silently building with the wrong
  # compiler than CI uses.
  toolchainFile = builtins.fromTOML (builtins.readFile (src + "/rust-toolchain.toml"));
  toolchainChannel = toolchainFile.toolchain.channel;

  rustPlatformFor =
    pkgs:
    if toolchainChannel == "stable" then
      pkgs.rustPlatform
    else
      throw ''
        helm: rust-toolchain.toml pins channel "${toolchainChannel}", but this
        flake only knows how to build the "stable" channel with nixpkgs'
        rustPlatform. Add a rust-overlay or fenix input and wire it in here
        rather than building with a different compiler than CI.
      '';

  # ── river ─────────────────────────────────────────────────────────────────
  # The river release helm's window-manager mapping is written and tested
  # against. Bump deliberately, never incidentally.
  riverTested = "0.4.8";

  # 0.4 is a hard floor, not a preference: river 0.3.x and river-classic
  # (0.3.17 in nixpkgs) keep window management *inside* the compositor and do
  # not implement river-window-management-v1, so helm-session has nothing to
  # drive and the desktop is inert. 0.5 is excluded because the protocol is
  # declared stable as of 0.4.0, but pre-1.0 and single-maintainer, so a minor
  # and helm should refuse to build rather than fail at runtime.
  #
  # NEEDS-HUMAN — how river is pinned. Two options:
  #   (a) what this file does: pin through the nixpkgs input. flake.lock fixes
  #       nixpkgs, which fixes river's tag, its source hash and its
  #       zig-dependency hash together, using nixpkgs' tested build recipe
  #       (verified: nixpkgs builds river 0.4.8 from
  #       https://codeberg.org/river/river tag v0.4.8 with zig 0.16).
  #       Cost: bumping river means bumping nixpkgs.
  #   (b) a dedicated tag-pinned input fed to `pkgs.river.overrideAttrs`.
  #       Cost: river's derivation carries TWO fixed-output hashes — `src.hash`
  #       and `zigDeps.hash` (the zig 0.16 package cache) — and the second
  #       cannot be derived from the tag by inspection. Someone with a working
  #       `nix build` must produce it; a guessed hash fails at install time.
  # Decide when we first need a river newer than nixpkgs carries.
  riverFor =
    pkgs:
    let
      v = pkgs.river.version;
    in
    if !(lib.versionAtLeast v "0.4" && lib.versionOlder v "0.5") then
      throw ''
        helm: nixpkgs provides river ${v}, but helm drives river over
        river-window-management-v1, which exists only in river 0.4.x. Pin
        nixpkgs to a revision carrying river 0.4.x, or set
        programs.helm.compositor to a river you have tested.
      ''
    else
      lib.warnIf (v != riverTested)
        "helm: river ${v} differs from the tested ${riverTested}; re-verify the window-manager mapping before shipping this combination."
        pkgs.river;

  # ── the desktop helm assembles itself out of ──────────────────────────────
  # Reused rather than rewritten (ADR 0007 / S8). Runtime dependencies of the
  # *desktop*, not build inputs of the crate.
  reusedTools =
    pkgs: with pkgs; [
      yazi # charon — file manager
      btop # horus — monitor
      starship # thoth — prompt
      zsh # thoth — shell
      fuzzel # hecate stopgap — launcher
      foot # terminal
    ];

  # Everything the session wrapper shells out to. Wrapping this onto PATH is
  # what makes the wrapper work on a NixOS box, where /usr/bin is empty. river
  # is here rather than left to the system profile because a helm package that
  # cannot start its compositor is not a desktop.
  wrapperRuntime =
    pkgs:
    (with pkgs; [
      coreutils # date, sleep, mkdir, timeout
      systemd # systemctl --user
      dbus # dbus-update-activation-environment, dbus-run-session
      glib # gsettings
    ])
    ++ [ (riverFor pkgs) ];
}
