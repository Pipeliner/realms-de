# realm — the reference build (ADR 0010).
#
# This flake lives at the repository root because a flake cannot reference
# sources above its own directory: rooted at packaging/nix/ it could never see
# ../../Cargo.toml. The parts that are logically packaging — the derivation, the
# two modules, the checks — stay in packaging/nix/ and are imported from here.
#
#   packaging/nix/support.nix              toolchain + river pins, shared tool lists
#   packaging/nix/package.nix              the derivation
#   packaging/nix/nixos-module.nix         programs.realm for NixOS
#   packaging/nix/home-manager-module.nix  programs.realm for home-manager
#   packaging/nix/checks.nix               shellcheck + the NixOS VM test
#
# ─────────────────────────────────────────────────────────────────────────────
# `flake.lock` is committed: it pins the declared flake inputs.
# Update it only through a deliberate `nix flake update` or `nix flake lock`,
# review the resulting revision/hash diff, and run the full flake checks. The
# lock is the reproducibility boundary, not a substitute for runtime evidence.
# ─────────────────────────────────────────────────────────────────────────────
#
# COMPOSITOR: river 0.4.x, not niri (ADR 0013 supersedes ADR 0002). river 0.4
# removed window management from the compositor and exposes it over
# `river-window-management-v1`; realm-session is the window manager that drives
# it. river is pinned and carried in realm's runtime closure — see
# packaging/nix/support.nix for the version guard and the pinning decision.
#
# PRE-ALPHA (0.1.0). The Cargo workspace builds realm-wm and realm-bar alongside
# the command-line and validation tools. Their installed live-session proof is
# still pending; source presence is not treated as a usable desktop claim.
{
  description = "realm — a keyboard-first, gapless-tiling Wayland desktop environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Deliberately no rust-overlay/fenix input: rust-toolchain.toml asks for
    # `stable`, which nixpkgs' own rustPlatform satisfies. The moment that file
    # pins an exact version, add fenix and honour it — see support.rustPlatformFor.
    #
    # Deliberately no separate river input either: pinning nixpkgs pins river's
    # tag, its source hash *and* its zig-dependency hash in one place. The
    # alternative, and why it needs a human, is documented in support.nix.
  };

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;

      # realm is a Wayland desktop: Linux only. Darwin would evaluate and could
      # never run, and a package that cannot run should not be offered.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      support = import ./packaging/nix/support.nix {
        inherit lib;
        src = self;
      };

      realmPackage =
        pkgs:
        import ./packaging/nix/package.nix {
          inherit pkgs lib support;
          src = self;
        };

      desktopAdmissionVmTest =
        pkgs:
        import ./packaging/nix/desktop-admission-vm-test.nix {
          inherit pkgs support;
          src = self;
        };

      nixosModule = import ./packaging/nix/nixos-module.nix { inherit self support; };
      homeManagerModule = import ./packaging/nix/home-manager-module.nix { inherit self support; };
    in
    {
      packages = forAllSystems (pkgs: rec {
        realm = realmPackage pkgs;
        default = realm;
      });

      apps = forAllSystems (pkgs: rec {
        realm-session = {
          type = "app";
          program = "${realmPackage pkgs}/bin/realm-session";
        };
        default = realm-session;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          name = "realm-dev";

          packages =
            (with pkgs; [
              # Toolchain — the same channel rust-toolchain.toml asks CI for.
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer

              # Packaging and validation, so the checks CI runs are runnable
              # locally: `shellcheck packaging/session/realm-session`,
              # `rpmspec -P packaging/fedora/realm.spec`, `dpkg-parsechangelog`.
              shellcheck
              rpm
              dpkg
              cargo-deb
              # cargo-generate-rpm is not in nixpkgs (checked against
              # nixos-unstable, 2026-08). packaging/fedora/realm.spec is the
              # supported rpm path; `cargo install cargo-generate-rpm` if you
              # want the metadata-driven one.
            ])
            ++ [
              # The same pinned river the package and the module use, so
              # `nix develop` and a real login disagree about nothing.
              (support.riverFor pkgs)
            ]
            ++ (with pkgs; [
              wayland-utils # `wayland-info` — what the compositor advertises
              wl-clipboard
            ])
            ++ support.reusedTools pkgs
            ++ support.wrapperRuntime pkgs;

          shellHook = ''
            echo "realm ${support.version} dev shell — pre-alpha: realm-core is the only crate that builds."
            echo "  cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test"
            echo "  ./packaging/session/realm-session --check   # session contract preflight"
          '';
        };
      });

      nixosModules = {
        realm = nixosModule;
        default = nixosModule;
      };

      homeManagerModules = {
        realm = homeManagerModule;
        default = homeManagerModule;
      };

      # `checks.session-boots` is a NixOS VM test and needs a KVM-capable
      # builder; `shellcheck`, `package`, `packaged-binaries`, and
      # `realm-sdd-git-runtime` build
      # anywhere. CI (distro.yml) falls back to `nix flake check --no-build`
      # plus those four when /dev/kvm is absent, so keep them independently
      # buildable.
      checks = forAllSystems (
        pkgs:
        import ./packaging/nix/checks.nix {
          inherit pkgs lib nixosModule;
          src = self;
          realm = realmPackage pkgs;
          desktopAdmissionVmTest = desktopAdmissionVmTest pkgs;
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style or pkgs.nixpkgs-fmt);
    };
}
