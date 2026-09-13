# Shared facts for realm's Nix build: which toolchain, which river, which tools.
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
        realm: rust-toolchain.toml pins channel "${toolchainChannel}", but this
        flake only knows how to build the "stable" channel with nixpkgs'
        rustPlatform. Add a rust-overlay or fenix input and wire it in here
        rather than building with a different compiler than CI.
      '';

  # ── river ─────────────────────────────────────────────────────────────────
  # The river release realm's window-manager mapping is written and tested
  # against. Bump deliberately, never incidentally.
  riverTested = "0.4.8";

  # 0.4 is a hard floor, not a preference: river 0.3.x and river-classic
  # (0.3.17 in nixpkgs) keep window management *inside* the compositor and do
  # not implement river-window-management-v1, so realm-session has nothing to
  # drive and the desktop is inert. 0.5 is excluded because the protocol is
  # declared stable as of 0.4.0, but pre-1.0 and single-maintainer, so a minor
  # and realm should refuse to build rather than fail at runtime.
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
        realm: nixpkgs provides river ${v}, but realm drives river over
        river-window-management-v1, which exists only in river 0.4.x. Pin
        nixpkgs to a revision carrying river 0.4.x, or set
        programs.realm.compositor to a river you have tested.
      ''
    else
      lib.warnIf (v != riverTested)
        "realm: river ${v} differs from the tested ${riverTested}; re-verify the window-manager mapping before shipping this combination."
        pkgs.river;

  # Realm's generated Yazi configuration targets the selected v25.4 schema.
  # Build that exact release from the retained, digest-checked source and
  # vendor archives; nixpkgs' moving Yazi is intentionally not substituted.
  yaziRetained = "25.4.8";
  yaziRetainedSource =
    pkgs:
    pkgs.runCommand "realm-yazi-${yaziRetained}-source"
      {
        nativeBuildInputs = [
          pkgs.gnutar
          pkgs.zstd
        ];
      }
      ''
        mkdir -p "$out"
        tar -xzf \
          ${src + "/packaging/tool-sources/bundles/yazi-25.4.8/source.tar.gz"} \
          -C "$out" --strip-components=1
        tar --zstd -xf \
          ${src + "/packaging/tool-sources/bundles/yazi-25.4.8/vendor.tar.zst"} \
          -C "$out"
        install -m 0644 \
          ${src + "/packaging/tool-sources/bundles/yazi-25.4.8/Cargo.lock"} \
          "$out/Cargo.lock"
      '';

  yaziUnwrappedFor =
    pkgs:
    (rustPlatformFor pkgs).buildRustPackage {
      pname = "yazi";
      version = yaziRetained;
      src = yaziRetainedSource pkgs;
      cargoVendorDir = "vendor";
      cargoBuildFlags = [
        "--package"
        "yazi-fm"
        "--package"
        "yazi-cli"
      ];
      strictDeps = true;
      preBuild = ''
        export CFLAGS="''${CFLAGS-} -std=gnu17"
      '';
      env = {
        SOURCE_DATE_EPOCH = "1744112829";
        VERGEN_GIT_SHA = "99ea3b74c4260a724b43af812df0f68ef59395b7";
        VERGEN_GIT_COMMIT_DATE = "2025-04-08";
        VERGEN_BUILD_DATE = "2025-04-08";
      };
      buildInputs = [ pkgs.rust-jemalloc-sys ];
      postInstall = ''
        install -Dm444 assets/yazi.desktop -t "$out/share/applications"
        install -Dm444 assets/logo.png "$out/share/pixmaps/yazi.png"
      '';
      meta = pkgs.yazi-unwrapped.meta // {
        changelog = "https://github.com/sxyazi/yazi/blob/v${yaziRetained}/CHANGELOG.md";
      };
    };

  yaziFor = pkgs: pkgs.yazi.override { yazi-unwrapped = yaziUnwrappedFor pkgs; };

  # ── the desktop realm assembles itself out of ──────────────────────────────
  # Reused rather than rewritten (ADR 0007 / S8). Runtime dependencies of the
  # *desktop*, not build inputs of the crate.
  reusedTools =
    pkgs: with pkgs; [
      (yaziFor pkgs) # charon — retained file manager
      btop # horus — monitor
      starship # thoth — prompt
      zsh # thoth — shell
      fuzzel # hecate stopgap — launcher
      foot # terminal
    ];

  # Everything the session wrapper shells out to. Wrapping this onto PATH is
  # what makes the wrapper work on a NixOS box, where /usr/bin is empty. river
  # is here rather than left to the system profile because a realm package that
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
