# A D-Bus/PipeWire client installed only in checks.session-boots. It keeps one
# bus connection alive for portal request/session ownership and maps a real
# GStreamer buffer from the restricted ScreenCast remote.
{ pkgs, lib, src }:
let
  python = pkgs.python3.withPackages (packages: [ packages.pygobject3 ]);
  giTypelibPath = lib.makeSearchPathOutput "out" "lib/girepository-1.0" [
    pkgs.glib
    pkgs.gst_all_1.gstreamer
    pkgs.gst_all_1.gst-plugins-base
  ];
  gstPluginPath = lib.makeSearchPath "lib/gstreamer-1.0" [
    pkgs.pipewire
    pkgs.gst_all_1.gst-plugins-base
  ];
in
pkgs.writeShellApplication {
  name = "realm-portal-vm";
  runtimeInputs = [ python ];
  text = ''
    export GI_TYPELIB_PATH=${lib.escapeShellArg giTypelibPath}
    export GST_PLUGIN_SYSTEM_PATH_1_0=${lib.escapeShellArg gstPluginPath}
    exec ${python}/bin/python3 ${src + "/packaging/nix/portal_vm_helper.py"} "$@"
  '';
}
