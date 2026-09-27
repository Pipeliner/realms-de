# Build-free evaluation of the real VM node, also usable by source-only tests.
let
  checks = import ./checks.nix {
    pkgs.testers.nixosTest = value: value;
    lib = {};
    src = ./.;
    realm = null;
    desktopAdmissionVmTest = null;
    nixosModule = "production-module";
    sourceRevision = "test";
    vmControlHelper = null;
    portalVmHelper = null;
  };
  node = checks.session-boots.nodes.machine { config = {}; pkgs = {}; };
in {
  options = node.virtualisation.qemu.options or [];
  params = node.boot.kernelParams;
  imports = node.imports;
  manager = node.systemd.settings.Manager or {};
}
