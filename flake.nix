# helm — the reference build (ADR 0010).
#
# This flake lives at the repository root because a flake cannot reference
# sources above its own directory: rooted at packaging/nix/ it could never see
# ../../Cargo.toml. The parts that are logically packaging — the derivation, the
# two modules, the checks — stay in packaging/nix/ and are imported from here.
#
#   packaging/nix/support.nix              toolchain + river pins, shared tool lists
#   packaging/nix/package.nix              the derivation
#   packaging/nix/nixos-module.nix         programs.helm for NixOS
#   packaging/nix/home-manager-module.nix  programs.helm for home-manager
#   packaging/nix/checks.nix               shellcheck + the NixOS VM test
#
# ─────────────────────────────────────────────────────────────────────────────
# NO flake.lock IS COMMITTED, and that matters: without it this is not a
# reproducible build, which is the entire claim ADR 0010 makes for it. It could
# not be generated here — `nix flake lock` resolves `github:` inputs through
# api.github.com, which this container's egress policy answers with 403. Run
#
#     nix flake lock
#
# once on any machine with network access and commit the result.
#
# What *was* verified, rather than assumed: a nix 2.24.9 binary was fetched and
# this flake was evaluated end to end with
#
#     nix flake check --no-build \
#       --override-input nixpkgs tarball+https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz
#
# against nixos-unstable (nixos-26.11pre1060451.56c02bc00adc, 2026-08-23).
# packages, devShells, both module evaluations, the shellcheck check and the VM
# test derivation all evaluate. Nothing was *built*: no rustc, no VM boot.
# Evaluation catches typos and dead attribute names, not runtime behaviour.
# ─────────────────────────────────────────────────────────────────────────────
#
# COMPOSITOR: river 0.4.x, not niri (ADR 0013 supersedes ADR 0002). river 0.4
# removed window management from the compositor and exposes it over
# `river-window-management-v1`; helm-session is the window manager that drives
# it. river is pinned and carried in helm's runtime closure — see
# packaging/nix/support.nix for the version guard and the pinning decision.
#
# PRE-ALPHA (0.1.0). The Cargo workspace contains one crate, helm-core, and it
# is a *library*: this package installs no helm binaries, because none exist
# yet. What it does install is real and testable today — the session wrapper,
# the wayland-session entry, the systemd user units and the palette. helm-bar,
# helm-sessiond and helm land in M1–M2 and will appear in $out/bin without any
# change to this file.
{
  description = "helm — a keyboard-first, gapless-tiling Wayland desktop environment";

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

      # helm is a Wayland desktop: Linux only. Darwin would evaluate and could
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

      helmPackage =
        pkgs:
        import ./packaging/nix/package.nix {
          inherit pkgs lib support;
          src = self;
        };

      nixosModule = import ./packaging/nix/nixos-module.nix { inherit self support; };
      homeManagerModule = import ./packaging/nix/home-manager-module.nix { inherit self support; };
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
              # Toolchain — the same channel rust-toolchain.toml asks CI for.
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
              (support.riverFor pkgs)
            ]
            ++ (with pkgs; [
              wayland-utils # `wayland-info` — what the compositor advertises
              wl-clipboard
            ])
            ++ support.reusedTools pkgs
            ++ support.wrapperRuntime pkgs;

          shellHook = ''
            echo "helm ${support.version} dev shell — pre-alpha: helm-core is the only crate that builds."
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

      # `checks.session-boots` is a NixOS VM test and needs a KVM-capable
      # builder; `shellcheck` and `package` build anywhere. CI (distro.yml)
      # falls back to `nix flake check --no-build` plus those two when /dev/kvm
      # is absent, so keep them independently buildable.
      checks = forAllSystems (
        pkgs:
        import ./packaging/nix/checks.nix {
          inherit pkgs lib nixosModule;
          src = self;
          helm = helmPackage pkgs;
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style or pkgs.nixpkgs-fmt);
    };
}
