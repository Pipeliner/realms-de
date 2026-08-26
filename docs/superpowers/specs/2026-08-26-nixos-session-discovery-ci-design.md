# NixOS session-discovery CI design

## Context

Issue #112 blocks every open pull request. The NixOS VM starts successfully,
but its first assertion expects `helm.desktop` in the system profile. That is
not the path NixOS promises for display-manager sessions: the module registers
a rewritten package through `services.displayManager.sessionPackages`, while
the fixture deliberately enables no display manager.

The package artifact exists. The failure is therefore a contract mismatch
between the test and NixOS session discovery, not a failed package install and
not a KVM failure.

## Decision

Helm's NixOS contract is the native `services.displayManager.sessionPackages`
interface. Helm must not enable or choose a display manager. The NixOS VM test
will enable one supported display-manager integration solely as a consumer of
that contract, then assert the registered session data contains `helm.desktop`
with `DesktopNames=helm` and an `Exec` pointing at the module-generated
launcher.

The system-profile path `/run/current-system/sw/share/wayland-sessions` is not
a public Helm contract and will not be forced into `environment.pathsToLink`.

## Remote CI contract

GitHub Actions remains the authoritative execution environment until a local
Nix version compatible with the flake is available. CI must never silently
pass without checking the buildable artifacts:

- With `/dev/kvm`, run the full `nix flake check`, including the NixOS VM.
- Without `/dev/kvm`, run evaluation plus independent `shellcheck` and package
  builds. The workflow must state that the VM test did not run.
- The VM assertion itself must exercise the selected NixOS session-discovery
  contract rather than a system-profile implementation detail.

This preserves a free non-KVM path for ordinary PR validation and makes KVM an
opportunistic, higher-fidelity gate instead of a prerequisite.

## Tests and evidence

Before implementation, change the VM assertion so the current implementation
fails for the right reason: it should require display-manager session data,
not a profile symlink. Then make the smallest module/fixture change needed for
that assertion to pass. The remote CI run must be inspected after push; local
success is not claimed where the host cannot evaluate the flake.

## Scope

This design addresses #112 only. It does not add CI providers, GitHub Projects,
daemons, cron jobs, or published orchestration infrastructure. It does not
bundle the separate Dependabot `toml` PR #11 rebase or the descriptor writer
#110.

## Relations

- amends [[SPEC 0005 — Session startup and desktop integration]]
- addresses [[GitHub issue 112]]
- informs [[GitHub issue 81]]
