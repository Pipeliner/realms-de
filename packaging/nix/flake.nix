# helm — the reference build (ADR 0010).
#
# ─────────────────────────────────────────────────────────────────────────────
# TWO THINGS TO KNOW BEFORE YOU RUN THIS
#
# 1. NO flake.lock IS COMMITTED. It could not be generated: the agent container
#    that wrote this file has no `nix` binary and no network access to
#    cache.nixos.org (see .claude/memory/20-environment.md). Run
#    `nix flake lock` once on a machine with nix and commit the result; until
#    then every evaluation resolves nixos-unstable afresh and is not
#    reproducible. `nix flake lock` needs api.github.com, which this container's
#    egress policy answers with 403 — that is the only reason the lock is
#    missing.
#
#    What *was* verified: a nix 2.24.9 binary was fetched and this file was
#    evaluated end to end with
#      nix flake check --no-build \
#        --override-input nixpkgs tarball+https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz
#    against nixos-unstable (nixos-26.11pre1060451.56c02bc00adc, 2026-08-23).
#    packages, devShells, both NixOS module evaluations, the shellcheck check and
#    the VM test derivation all evaluate. Nothing has been *built*: no rustc, no
#    VM boot. Evaluation catches typos and dead attribute names, not runtime.
#
# 2. A FLAKE CANNOT REFERENCE SOURCES ABOVE ITS OWN DIRECTORY. This file lives
#    in packaging/nix/ for tidiness, but a flake rooted there cannot see
#    ../../Cargo.toml, so `nix build ./packaging/nix#default` will fail with the
#    error thrown by `helmSrc` below. Land it at the repo root — `git mv
#    packaging/nix/flake.nix flake.nix` — which is a one-line move the repo
#    owner (not this file's author) makes; root files belong to another agent.
#    Everything below refers to files by `helmSrc + "/path"` rather than by
#    relative path, so the move needs no other edit.
# ─────────────────────────────────────────────────────────────────────────────
#
# COMPOSITOR: river 0.4.x, not niri (ADR 0013 supersedes ADR 0002). river 0.4
# removed window management from the compositor and exposes it over
# `river-window-management-v1`; helm-session is the window manager that drives
# it. Consequences for this file, in order of how much they will surprise you:
#
#   * `programs.helm.compositor` defaults to a pinned river 0.4.x, and river is
#     in the helm package's runtime closure. A river older than 0.4 does not
#     speak the protocol at all — hence the version guard in `riverFor`.
#   * The pin is the nixpkgs input plus flake.lock. That is a rev pin: locking
#     nixpkgs locks river's source tag, its hash, and its zig dependency hash
#     together. See the NEEDS-HUMAN block on `riverFor` for the alternative
#     (a dedicated tag input) and why it is not the default.
#
# PRE-ALPHA (0.1.0). The Cargo workspace currently contains one crate,
# helm-core, and it is a *library*: this package installs no helm binaries,
# because none exist yet. What it does install is real and testable today — the
# session wrapper, the wayland-session entry, the systemd user units and the
# palette. helm-bar, helm-sessiond and helm land in M1–M2 and will appear in
# $out/bin without any change to this file.
{
  description = "helm — a keyboard-first, gapless-tiling Wayland desktop environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Deliberately no rust-overlay/fenix input: rust-toolchain.toml asks for
    # `stable`, which nixpkgs' own rustPlatform satisfies. The moment that file
    # pins an exact version, add fenix and honour it — see rustPlatformFor.
  };

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;

      version = "0.1.0";

      # Linux only: helm is a Wayland desktop. Darwin would evaluate but could
      # never run, and a package that cannot run should not be offered.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # The workspace root. See note 2 in the header: this is `self` because the
      # flake is meant to sit at the repo root. The throw is deliberate — a
      # confusing "file not found" three layers into buildRustPackage is worse
      # than one sentence saying exactly what is wrong.
      helmSrc =
        if builtins.pathExists (self + "/Cargo.toml") then
          self
        else
          throw ''
            helm flake: no Cargo.toml next to flake.nix.

            A flake cannot reference sources above its own directory, so this
            file must live at the repository root. Move it there:

                git mv packaging/nix/flake.nix flake.nix && nix flake lock
          '';

      # Honour rust-toolchain.toml rather than assuming. `channel = "stable"`
      # maps onto nixpkgs' default rustPlatform; anything else needs a real
      # toolchain input and should say so loudly instead of silently building
      # with the wrong compiler.
      toolchainFile = builtins.fromTOML (builtins.readFile (helmSrc + "/rust-toolchain.toml"));
      toolchainChannel = toolchainFile.toolchain.channel;
      rustPlatformFor =
        pkgs:
        if toolchainChannel == "stable" then
          pkgs.rustPlatform
        else
          throw ''
            helm flake: rust-toolchain.toml pins channel "${toolchainChannel}",
            but this flake only knows how to build the "stable" channel with
            nixpkgs' rustPlatform. Add a rust-overlay or fenix input and wire it
            in here rather than building with a different compiler than CI.
          '';

      # The river release helm's protocol mapping is written and tested against.
      # Bump deliberately, never incidentally.
      riverTested = "0.4.8";

      # river, version-guarded.
      #
      # 0.4 is a hard floor, not a preference: river 0.3.x and river-classic
      # (0.3.17 in nixpkgs) have window management *inside* the compositor and
      # do not implement river-window-management-v1, so helm-session has nothing
      # to drive and the desktop is inert. 0.5 is excluded because the protocol
      # is registry-classified unstable — a minor bump is exactly where it may
      # move, and helm should refuse to build rather than fail at runtime.
      #
      # NEEDS-HUMAN — how river is pinned. Two options:
      #   (a) what this file does: pin through the nixpkgs input. flake.lock
      #       fixes nixpkgs, which fixes river's tag, its source hash and its
      #       zig-dependency hash in one place, using nixpkgs' tested build
      #       recipe. Cost: bumping river means bumping nixpkgs.
      #   (b) a dedicated tag-pinned input,
      #       `https://codeberg.org/river/river/archive/refs/tags/vX.Y.Z.tar.gz`
      #       (the upstream URL nixpkgs itself fetches), fed to
      #       `pkgs.river.overrideAttrs`. Cost: river's derivation carries TWO
      #       fixed-output hashes — `src.hash` and `zigDeps.hash` (zig 0.16
      #       package cache) — and the second cannot be derived from the tag.
      #       Someone with a nix build must produce it. Guessing it produces a
      #       hash-mismatch build failure at install time, which is why (b) is
      #       not the default.
      # Decide when we first need a river newer than nixpkgs carries.
      riverFor =
        pkgs:
        let
          v = pkgs.river.version;
        in
        if !(lib.versionAtLeast v "0.4" && lib.versionOlder v "0.5") then
          throw ''
            helm flake: nixpkgs provides river ${v}, but helm drives river over
            river-window-management-v1, which exists only in river 0.4.x.
            Pin nixpkgs to a revision carrying river 0.4.x, or set
            programs.helm.compositor to a river you have tested.
          ''
        else
          lib.warnIf (v != riverTested)
            "helm: river ${v} differs from the tested ${riverTested}; river-window-management-v1 is classified unstable, so re-test the window-manager mapping."
            pkgs.river;

      # The tools helm reuses rather than rewrites (ADR 0007 / S8). These are
      # runtime dependencies of the *desktop*, not build inputs of the crate.
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
      # what makes the wrapper work on a NixOS box, where /usr/bin is empty.
      # river is here rather than left to the system profile because a helm
      # package that cannot start its compositor is not a desktop.
      wrapperRuntime =
        pkgs:
        (with pkgs; [
          coreutils # date, sleep, mkdir
          systemd # systemctl --user
          dbus # dbus-update-activation-environment
          glib # gsettings
        ])
        ++ [ (riverFor pkgs) ];

      helmPackage =
        pkgs:
        (rustPlatformFor pkgs).buildRustPackage {
          pname = "helm";
          inherit version;

          src = lib.cleanSourceWith {
            src = helmSrc;
            # target/ and result/ are large and irrelevant; keeping them out
            # keeps the store path stable across local builds.
            filter =
              path: type:
              let
                base = baseNameOf path;
              in
              !(builtins.elem base [
                "target"
                "result"
                ".direnv"
              ]);
          };

          cargoLock.lockFile = helmSrc + "/Cargo.lock";

          nativeBuildInputs = [ pkgs.makeWrapper ];

          # helm-core's tests include the palette lint, so a palette that fails
          # WCAG floors fails the build. That is the intended behaviour.
          doCheck = true;

          postInstall = ''
            install -Dm755 ${helmSrc + "/packaging/session/helm-session"} \
              $out/bin/helm-session

            # The desktop entry must point at the store path, not /usr/bin.
            install -Dm644 ${helmSrc + "/packaging/session/helm.desktop"} \
              $out/share/wayland-sessions/helm.desktop
            substituteInPlace $out/share/wayland-sessions/helm.desktop \
              --replace-fail /usr/bin/helm-session $out/bin/helm-session

            for unit in ${helmSrc + "/packaging/systemd"}/*; do
              install -Dm644 "$unit" $out/lib/systemd/user/"$(basename "$unit")"
            done
            # The units name /usr/bin paths that do not exist on NixOS. They are
            # rewritten here rather than in the unit files themselves so the deb
            # and rpm keep working unchanged.
            substituteInPlace $out/lib/systemd/user/*.service \
              --replace-quiet /usr/bin/ $out/bin/

            install -Dm644 ${helmSrc + "/palette.toml"} $out/share/helm/palette.toml

            wrapProgram $out/bin/helm-session \
              --prefix PATH : ${lib.makeBinPath (wrapperRuntime pkgs)}
          '';

          meta = {
            description = "Keyboard-first, gapless-tiling Wayland desktop environment";
            homepage = "https://github.com/pipeliner/realms-de";
            license = with lib.licenses; [
              mit
              asl20
            ];
            platforms = lib.platforms.linux;
            mainProgram = "helm-session";
          };
        };

      # ── NixOS module ───────────────────────────────────────────────────────
      nixosModule =
        {
          config,
          pkgs,
          lib,
          ...
        }:
        let
          cfg = config.programs.helm;

          # The compositor is chosen per-host, so the launch command is built
          # here rather than baked into the package.
          launcher = pkgs.writeShellScript "helm-session-launch" ''
            export HELM_COMPOSITOR=${lib.getExe cfg.compositor}
            export HELM_CURSOR_THEME=${lib.escapeShellArg cfg.cursorTheme}
            export HELM_CURSOR_SIZE=${toString cfg.cursorSize}
            exec ${cfg.package}/bin/helm-session "$@"
          '';

          # `providedSessions` is what services.displayManager.sessionPackages
          # checks; without it NixOS refuses the package.
          sessionPackage =
            pkgs.runCommand "helm-session-entry"
              {
                passthru.providedSessions = [ "helm" ];
              }
              ''
                install -Dm644 ${cfg.package}/share/wayland-sessions/helm.desktop \
                  $out/share/wayland-sessions/helm.desktop
                substituteInPlace $out/share/wayland-sessions/helm.desktop \
                  --replace-fail ${cfg.package}/bin/helm-session ${launcher}
              '';
        in
        {
          options.programs.helm = {
            enable = lib.mkEnableOption "the helm desktop environment";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "helm.packages.\${system}.default";
              description = "The helm package to install.";
            };

            compositor = lib.mkOption {
              type = lib.types.package;
              default = riverFor pkgs;
              defaultText = lib.literalExpression "pkgs.river (0.4.x, version-guarded)";
              description = ''
                The Wayland compositor helm runs on: river 0.4.x, driven by
                helm-session over river-window-management-v1 (ADR 0013).

                Point this elsewhere and you own the consequences — a compositor
                that does not implement river-window-management-v1 leaves helm
                with no way to place windows. The seam is real (helm-session
                talks to a WmBackend, not to river), but there is exactly one
                backend today.
              '';
            };

            paletteFile = lib.mkOption {
              type = lib.types.path;
              default = "${cfg.package}/share/helm/palette.toml";
              defaultText = lib.literalExpression "\${cfg.package}/share/helm/palette.toml";
              description = ''
                System-wide palette, installed to /etc/helm/palette.toml. Every
                themed surface is generated from this one file (ADR 0005); a
                user's own ~/.config/helm/palette.toml takes precedence.
              '';
            };

            cursorTheme = lib.mkOption {
              type = lib.types.str;
              default = "Adwaita";
              description = ''
                Cursor theme. Set in the session environment *and* in gsettings
                by the session wrapper, because a cursor theme set in only one
                of the two changes size as the pointer crosses windows
                (docs/PITFALLS.md, "Cursor theme unset").
              '';
            };

            cursorSize = lib.mkOption {
              type = lib.types.int;
              default = 24;
              description = "Cursor size in logical pixels.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [
              cfg.package
              cfg.compositor
              sessionPackage
            ] ++ reusedTools pkgs;

            # Registers helm in the display manager's session list.
            services.displayManager.sessionPackages = [ sessionPackage ];

            # Picks up lib/systemd/user/*.{target,service} from the package.
            systemd.packages = [ cfg.package ];

            environment.etc."helm/palette.toml".source = cfg.paletteFile;

            # ── the session-environment handshake (ADR 0011) ────────────────
            #
            # The wrapper does the real work: it waits for WAYLAND_DISPLAY to
            # exist, then imports it into systemd --user *and* the D-Bus
            # activation environment before any client starts. What the module
            # can add is making sure both channels exist at all — a user D-Bus
            # bus (dbus.implementation = "broker" ships one), a systemd user
            # manager, and dconf so the gsettings half of the cursor contract
            # has somewhere to write.
            #
            # XDG_CURRENT_DESKTOP / XDG_SESSION_TYPE are deliberately NOT set in
            # environment.sessionVariables: they must describe *this* session,
            # and a machine that also offers GNOME must not have every session
            # claiming to be helm. The wrapper exports them per-session instead.
            services.dbus.enable = true;
            programs.dconf.enable = true;
            security.polkit.enable = true;
            hardware.graphics.enable = lib.mkDefault true;

            # Portals: a browser with no portal backend has a silently broken
            # "Open File" (docs/PITFALLS.md). gtk is the pragmatic default —
            # it implements FileChooser and Settings; add wlr for screencast
            # under niri if you need it.
            xdg.portal = {
              enable = true;
              extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
              config.helm.default = [ "gtk" ];
              xdgOpenUsePortal = true;
            };

            # The glyph contract (ADR 0012). IBM Plex Mono is the design's face;
            # the symbols-only Nerd Font supplies runes and instrument glyphs.
            # Without these the bar is a row of tofu on first boot.
            # On nixpkgs older than 24.11 this attribute is
            # `(nerdfonts.override { fonts = [ "NerdFontsSymbolsOnly" ]; })`.
            fonts.packages = [
              pkgs.ibm-plex
              pkgs.nerd-fonts.symbols-only
            ];

            # XWayland: old apps should look wrong-ish, not broken.
            programs.xwayland.enable = lib.mkDefault true;
          };
        };

      # ── home-manager module ────────────────────────────────────────────────
      homeManagerModule =
        {
          config,
          pkgs,
          lib,
          ...
        }:
        let
          cfg = config.programs.helm;
          shipped = name: helmSrc + "/configs/${name}";
          # configs/ is filled in by M1's theming pipeline; link only what is
          # actually there so a fresh clone does not fail to evaluate.
          linkIfPresent =
            target: name:
            lib.optionalAttrs (builtins.pathExists (shipped name)) {
              ${target}.source = shipped name;
            };
        in
        {
          options.programs.helm = {
            enable = lib.mkEnableOption "helm user configuration";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "helm.packages.\${system}.default";
              description = "The helm package whose units and wrapper this user runs.";
            };

            paletteFile = lib.mkOption {
              type = lib.types.path;
              default = "${cfg.package}/share/helm/palette.toml";
              defaultText = lib.literalExpression "\${cfg.package}/share/helm/palette.toml";
              description = ''
                This user's palette, linked to ~/.config/helm/palette.toml.
                `helm ctl theme apply` (M1) renders every themed file from it.
              '';
            };

            startBar = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = ''
                Run helm-bar as a user unit under helm-session.target. The unit
                is installed either way; this only controls whether it is wanted
                by the session target.
              '';
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ] ++ reusedTools pkgs;

            xdg.configFile =
              {
                "helm/palette.toml".source = cfg.paletteFile;
              }
              # M1 replaces these static links with helm-theme's generated
              # output; the destination paths do not change.
              // linkIfPresent "yazi" "yazi"
              // linkIfPresent "btop/themes/helm.theme" "btop/helm.theme"
              // linkIfPresent "starship.toml" "starship/starship.toml"
              // linkIfPresent "foot/foot.ini" "foot/foot.ini"
              // linkIfPresent "fuzzel/fuzzel.ini" "fuzzel/fuzzel.ini";

            # These mirror packaging/systemd/*. home-manager writes its own copy
            # into ~/.config/systemd/user, which takes precedence over the
            # package's units — keep the two in step.
            systemd.user.targets.helm-session = {
              Unit = {
                Description = "helm desktop session";
                BindsTo = [ "graphical-session.target" ];
                Wants = [ "graphical-session-pre.target" ];
                After = [ "graphical-session-pre.target" ];
              };
            };

            systemd.user.services.helm-daemon = {
              Unit = {
                Description = "helm session daemon";
                PartOf = [ "graphical-session.target" ];
                After = [ "graphical-session.target" ];
                ConditionEnvironment = "WAYLAND_DISPLAY";
              };
              Service = {
                # PRE-ALPHA: this binary does not exist yet (M2). The unit is
                # here so the session shape is complete and testable.
                ExecStart = "${cfg.package}/bin/helm-sessiond";
                Restart = "on-failure";
                RestartSec = 1;
              };
              Install.WantedBy = [ "helm-session.target" ];
            };

            systemd.user.services.helm-bar = {
              Unit = {
                Description = "helm bar";
                PartOf = [ "graphical-session.target" ];
                After = [
                  "graphical-session.target"
                  "helm-daemon.service"
                ];
                Wants = [ "helm-daemon.service" ];
                ConditionEnvironment = "WAYLAND_DISPLAY";
              };
              Service = {
                # PRE-ALPHA: lands in M2, like helm-sessiond above.
                ExecStart = "${cfg.package}/bin/helm-bar";
                # A crashed bar restarts itself and never takes the session with
                # it (docs/PITFALLS.md).
                Restart = "on-failure";
                RestartSec = 1;
              };
              Install.WantedBy = lib.optionals cfg.startBar [ "helm-session.target" ];
            };
          };
        };

      # ── checks ─────────────────────────────────────────────────────────────
      sessionBootTest =
        pkgs:
        # `pkgs.testers.nixosTest`, not the old top-level `nixosTest` alias, which
        # nixpkgs now refuses.
        pkgs.testers.nixosTest {
          name = "helm-session-boots";

          nodes.machine =
            { ... }:
            {
              imports = [ nixosModule ];
              programs.helm.enable = true;
              # Keep the VM small: no display manager, no graphical login. This
              # test asserts the *contract* is installed, not that a desktop
              # renders — see the M2 block below for that.
              virtualisation.memorySize = 2048;
            };

          testScript = ''
            # Everything asserted here is true of helm 0.1.0 today.
            machine.wait_for_unit("multi-user.target")

            # The wayland-session entry a display manager would offer.
            machine.succeed("test -f /run/current-system/sw/share/wayland-sessions/helm.desktop")
            machine.succeed("grep -q '^DesktopNames=helm$' /run/current-system/sw/share/wayland-sessions/helm.desktop")

            # The session wrapper is installed, wrapped, and its preflight
            # agrees that the compositor and the D-Bus tooling are present.
            machine.succeed("helm-session --version")
            machine.succeed("helm-session --check")

            # river 0.4.x, the compositor helm drives. `-version` (one dash) is
            # river's own spelling. Anything below 0.4 does not implement
            # river-window-management-v1, and the flake would have refused to
            # evaluate — this asserts the package that actually landed.
            machine.succeed("river -version")

            # The user units that carry the session.
            machine.succeed("test -f /etc/systemd/user/helm-session.target")
            machine.succeed("test -f /etc/systemd/user/helm-bar.service")
            machine.succeed("test -f /etc/systemd/user/helm-daemon.service")

            # The palette every themed surface is generated from.
            machine.succeed("test -f /etc/helm/palette.toml")

            # ── PENDING M2 — the assertion this test exists for ──────────────
            # helm-bar does not exist yet, so this block is not run. When M2
            # lands, delete `pending_m2` and the guard; the body is the real
            # test: log a user in, wait for the bar to map a layer surface, and
            # assert the environment handshake actually happened.
            pending_m2 = False
            if pending_m2:
                machine.succeed("loginctl enable-linger alice")
                machine.wait_for_unit("helm-session.target", user="alice")
                machine.wait_until_succeeds("pgrep -u alice helm-bar")
                # The failure this guards: a portal activated without
                # WAYLAND_DISPLAY hangs for 25 s and then fails.
                machine.succeed(
                    "su - alice -c 'systemctl --user show-environment' | grep -q '^WAYLAND_DISPLAY='"
                )
                machine.screenshot("helm-bar")
          '';
        };

      shellcheckCheck =
        pkgs:
        pkgs.runCommand "helm-shellcheck" { nativeBuildInputs = [ pkgs.shellcheck ]; } ''
          shellcheck --shell=bash ${helmSrc + "/packaging/session/helm-session"}
          touch $out
        '';
    in
    {
      packages = forAllSystems (pkgs: rec {
        helm = helmPackage pkgs;
        default = helm;
      });

      apps = forAllSystems (pkgs: rec {
        helm-session = {
          type = "app";
          program = "${helmPackage pkgs}/bin/helm-session";
        };
        default = helm-session;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          name = "helm-dev";

          packages =
            (with pkgs; [
              # Toolchain — the same one rust-toolchain.toml asks CI for.
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer

              # Packaging and validation, so the checks CI runs are runnable
              # locally: `shellcheck packaging/session/helm-session`,
              # `rpmspec -P packaging/fedora/helm.spec`, `dpkg-parsechangelog`.
              shellcheck
              rpm
              dpkg
              cargo-deb
              # cargo-generate-rpm is not in nixpkgs (checked against
              # nixos-unstable, 2026-08). packaging/fedora/helm.spec is the
              # supported rpm path; `cargo install cargo-generate-rpm` if you
              # want the metadata-driven one.
            ])
            ++ [
              # The same pinned river the package and the module use, so
              # `nix develop` and a real login disagree about nothing.
              (riverFor pkgs)
            ]
            ++ (with pkgs; [
              # The desktop helm assembles itself out of, so `nix develop`
              # gives a runnable session and not just a compiler.
              wayland-utils
              wl-clipboard
            ])
            ++ reusedTools pkgs
            ++ wrapperRuntime pkgs;

          shellHook = ''
            echo "helm ${version} dev shell — pre-alpha: helm-core is the only crate that builds."
            echo "  cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test"
            echo "  ./packaging/session/helm-session --check   # session contract preflight"
          '';
        };
      });

      nixosModules = {
        helm = nixosModule;
        default = nixosModule;
      };

      homeManagerModules = {
        helm = homeManagerModule;
        default = homeManagerModule;
      };

      # `nix flake check` needs a KVM-capable builder for the VM test; without
      # one, run `nix build .#checks.x86_64-linux.shellcheck` alone.
      checks = forAllSystems (pkgs: {
        shellcheck = shellcheckCheck pkgs;
        session-boots = sessionBootTest pkgs;
        package = helmPackage pkgs;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style or pkgs.nixpkgs-fmt);
    };
}
