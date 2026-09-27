# Explicitly built comparison fixture; ordinary `nix flake check` never builds it.
{ pkgs, src }:
let
  config = pkgs.writeText "realm-waybar-comparison.json" (
    builtins.toJSON {
      position = "bottom";
      layer = "top";
      exclusive = true;
      "modules-left" = [ "custom/realm" ];
      "modules-right" = [ "clock" "cpu" "memory" "network" "battery" ];
      "custom/realm" = {
        exec = "${pkgs.python3}/bin/python3 ${src + "/packaging/nix/waybar_compare.py"} --trace /run/user/1000/waybar-comparison/adapter.jsonl";
        "return-type" = "json";
        format = "{text}";
        escape = true;
        tooltip = true;
        # No interval or signal: Waybar consumes the adapter's event stream.
      };
      clock = { interval = 1; format = "{:%H:%M:%S}"; };
      cpu = { interval = 1; format = "cpu {usage}%"; };
      memory = { interval = 1; format = "mem {percentage}%"; };
      network = {
        interval = 1;
        "format-ethernet" = "net {ifname}";
        "format-wifi" = "net {essid}";
        "format-disconnected" = "net down";
      };
      battery = { interval = 1; format = "bat {capacity}%"; };
    }
  );
in
pkgs.runCommand "realm-waybar-comparison-fixture" { nativeBuildInputs = [ pkgs.python3 ]; } ''
  ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_waybar_compare.py"}
  ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_waybar_comparison_vm.py"}
  mkdir -p "$out"
  cp ${config} "$out/config.json"
''
