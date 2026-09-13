# SPEC 0026 — Ubuntu Noble private River closure probe

- **Status:** Accepted (2026-09-13)
- **Milestone:** M3
- **Issue:** [#98](https://github.com/Pipeliner/realms-de/issues/98)
- **Decisions:** [ADR 0022](../adr/0022-ubuntu-river-private-closure.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md)
- **Supersedes / Superseded by:** Refines SPEC 0019's explicitly separate
  River/wlroots packaging decision for Ubuntu only

## Purpose

Prove on Ubuntu 24.04 that Realm's selected River 0.4.8 can be built with a
complete private dependency closure carrying real DRM, input, rendering,
session and Xwayland support. The proof closes the source/build feasibility
question before the closure is integrated into the Debian package.

## Scope

**In:** exact source identities, a manifest validator, explicit networked
acquisition, network-disabled native compilation on Ubuntu 24.04, a private
relocatable runtime tree, wlroots feature assertions, ELF resolution and the
River version probe.

**Out:** local native builds or toolchain installation; Debian package
integration; clean installation; a display-manager or graphical login; a real
DRM-device launch; Fedora and Nix changes; public repositories, mirrors,
signing, uploaded binary artifacts or automatic dependency updates.

## Selected source authority

The selected runtime and compositor versions match nixpkgs revision
`9fbb54b33e91ee4ca368e35a78e0613c720600b3`, which is fixed by the repository's
`flake.lock`. The CI acquisition manifest records the URLs and raw archive
SHA-256 values below. Nix's normalized source hashes remain corroborating
recipe evidence; they do not replace the raw archive hashes consumed by this
probe. Meson 1.4.0 is the exact build-tool floor declared by the selected
libxkbcommon source; Noble's Meson 1.3.2 is insufficient.

| Input | Version / identity | Upstream archive | SHA-256 | Role |
|---|---|---|---|---|
| Zig x86_64 Linux | 0.16.0 | `https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz` | `70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00` | acquisition and River build only |
| Meson | 1.4.0 | `https://github.com/mesonbuild/meson/releases/download/1.4.0/meson-1.4.0.tar.gz` | `8fd6630c25c27f1489a8a0392b311a60481a3c161aa699b330e25935b750138d` | build tool only |
| Wayland | 1.26.0 | `https://gitlab.freedesktop.org/wayland/wayland/-/releases/1.26.0/downloads/wayland-1.26.0.tar.xz` | `64176eaa46e4969903e286f8e5ef8331affc17fdf03ac9b58381d2b23162b7a3` | private runtime and `wayland-scanner` |
| wayland-protocols | 1.49 | `https://gitlab.freedesktop.org/wayland/wayland-protocols/-/releases/1.49/downloads/wayland-protocols-1.49.tar.xz` | `ec4c8f74942d6dff7ace8b4ce4764f0ef9ff618a935d974ea77edee2ad240b14` | build-time protocol data |
| libdrm | 2.4.134 | `https://dri.freedesktop.org/libdrm/libdrm-2.4.134.tar.xz` | `ac5e74d157830eb8bee44c6a6bf3ad49774ef0dd2a72bdad74a8f20308b52a95` | private runtime |
| libinput | 1.31.3 | `https://gitlab.freedesktop.org/libinput/libinput/-/archive/1.31.3/libinput-1.31.3.tar.gz` | `b6749bf6f1890f6631c0a70a027c35fec9d2e096a39f720548896e41474a9854` | private input runtime |
| pixman | 0.46.4 | `https://cairographics.org/releases/pixman-0.46.4.tar.gz` | `d09c44ebc3bd5bee7021c79f922fe8fb2fb57f7320f55e97ff9914d2346a591c` | private runtime |
| libxkbcommon | 1.13.2 | `https://github.com/xkbcommon/libxkbcommon/archive/refs/tags/xkbcommon-1.13.2.tar.gz` | `acc4d5f7c3cbba5f9f8d08d8bdbeede84ecede46792f47929aa9321873385528` | private runtime; satisfies River's stricter 1.12 floor |
| libdisplay-info | 0.4.0 | `https://gitlab.freedesktop.org/emersion/libdisplay-info/-/archive/0.4.0/libdisplay-info-0.4.0.tar.gz` | `787b58aec473830b0030251ba2b880560b4b3853dc5d6961a6e9701abae29b55` | private DRM runtime |
| wlroots | 0.20.2 | `https://gitlab.freedesktop.org/wlroots/wlroots/-/archive/0.20.2/wlroots-0.20.2.tar.gz` | `972c7ac44b17828f4702bfae7cd8347346a3fb5b2c1076cfa2c3fcedac5ec343` | private compositor runtime |
| River | tag `v0.4.8`, commit `c4b5f706314555f4846e25b8d3635631387b3fdd` | `https://codeberg.org/river/river/releases/download/v0.4.8/river-0.4.8.tar.gz` | `6d4030526e307e40de357167b4d6daacb583aed353dd93e32e1314c2d34400fa` | selected compositor |

River's five direct Zig package-manager inputs come from the selected release's
`build.zig.zon` and are explicit manifest records:

| Package | Acquisition URL | Zig content hash |
|---|---|---|
| pixman | `https://codeberg.org/ifreund/zig-pixman/archive/v0.3.0.tar.gz` | `pixman-0.3.0-LClMnz2VAAAs7QSCGwLimV5VUYx0JFnX5xWU6HwtMuDX` |
| wayland | `https://codeberg.org/ifreund/zig-wayland/archive/v0.6.0.tar.gz` | `wayland-0.6.0-lQa1kqz8AQADQmdNJsNhLoNHcnEGEUjrOaPV-dtEnEmX` |
| wlroots | `https://codeberg.org/ifreund/zig-wlroots/archive/v0.20.1.tar.gz` | `wlroots-0.20.1-jmOlcqNVBAB3uB5oqBTzpRlwu-FmMyyZMVAWCe5kmcSt` |
| xkbcommon | `https://codeberg.org/ifreund/zig-xkbcommon/archive/v0.4.0.tar.gz` | `xkbcommon-0.4.0-VDqIe0i2AgDRsok2GpMFYJ8SVhQS10_PI2M_CnHXsJJZ` |
| translate-c | `git+https://codeberg.org/ziglang/translate-c/#57c559cf581b1fcad90494eda219f98abeb155ce` | `translate_c-0.0.0-Q_BUWlX1BgCD1wo6uo97prlp9VJ4gxAjwN_vZ7nsSjGN` |

Their pinned `build.zig.zon` files add two transitive records. translate-c
selects Aro, and zig-wlroots 0.20.1 selects zig-xkbcommon 0.3.0 in addition to
River's direct 0.4.0 selection. Both transitive packages declare an empty
dependency set, so these seven records are the complete recursive Zig package
graph:

| Package role | Acquisition URL | Zig content hash |
|---|---|---|
| translate-c → aro | `git+https://github.com/Vexu/arocc#5f5a050569a95ecc40a426f0c3666ae7ef987ede` | `aro-0.0.0-JSD1Qi7QNgDnfcrdEJf82v3o6MhZySjYVrtdfEf3E4Se` |
| zig-wlroots → xkbcommon 0.3.0 | `https://codeberg.org/ifreund/zig-xkbcommon/archive/v0.3.0.tar.gz` | `xkbcommon-0.3.0-VDqIe3K9AQB2fG5ZeRcMC9i7kfrp5m2rWgLrmdNn9azr` |

The validator rejects an absent or extra selected input, duplicate identity,
an archive URL that is not HTTPS, a Zig dependency URL that is neither HTTPS
nor `git+https`, a non-lowercase 64-digit archive digest, a malformed Zig
content hash, or a manifest nixpkgs revision different from the lock. Archive
bytes are hashed again before extraction and before network-disabled
compilation.

## Noble system boundary

Noble supplies runtime dependencies whose versions already meet the selected
sources' requirements: libseat 0.8.0, Mesa EGL/GLES/GBM, XCB/Xfixes 1.15 and
Xwayland 23.2.6. It also supplies Ninja and Python for the selected Meson 1.4.0
source entry point. The CI job installs their development
closure: build-essential, pkg-config, ninja-build, Python, bison, patchelf,
libffi, expat, libpciaccess, pthread stubs, udev, libevdev, mtdev, libwacom,
libcap, GL/EGL/GLES, GBM, X11, the XCB composite/EWMH/ICCCM/render/res/xfixes
packages, hwdata, xkb-data and Xwayland.

The private source set is required because Noble remains below these floors:

| Dependency | Required by selected source | Noble |
|---|---:|---:|
| Wayland | 1.24.0 | 1.22.0 |
| wayland-protocols | 1.47 | 1.45 |
| libdrm | 2.4.129 | 2.4.125 |
| libinput | 1.27 (River input configuration API) | 1.25.0 |
| pixman | 0.43.0 | 0.42.2 |
| libxkbcommon | 1.12.0 (River) | 1.6.0 |
| libdisplay-info | 0.2.0 | 0.1.1 |
| wlroots | 0.20.x | 0.17.1 |

The first feature-bearing River compile against Noble's libinput 1.25.0 is the
direct RED evidence for the libinput gap: translation of River's input code
failed because `libinput_device_config_3fg_drag_get_finger_count` and
`LIBINPUT_CONFIG_DRAG_LOCK_ENABLED_TIMEOUT` were absent. The former API is
marked `since 1.27` by the selected libinput headers. The closure therefore
uses nixpkgs' selected libinput 1.31.3 without patching River or removing input
configuration behavior.

All dependencies enabled in that libinput build are satisfied by Noble:

| Enabled libinput dependency | libinput 1.31.3 floor | Noble package version |
|---|---:|---:|
| libudev | no explicit version floor | 255.4-1ubuntu8.17 |
| libevdev | 1.10.0 | 1.13.1+dfsg-1build1 |
| mtdev | 1.1.0 | 1.1.6-1.1build1 |
| libwacom | 0.27 | 2.10.0-2 |

## Acquisition and build contract (L4)

1. The acquisition command validates the checked-in manifest, creates a new
   empty cache, downloads every archive to its canonical manifest filename,
   and verifies its SHA-256 before it may be used. It extracts the selected Zig
   binary only after its checksum passes.
2. Zig 0.16 `fetch` requires a project root and writes unpacked packages to its
   project-local `zig-pkg/` directory. Acquisition therefore creates one
   dedicated cache-local fetch root containing only a minimal `build.zig`, runs
   `zig fetch --global-cache-dir <cache>/zig-global` from that root once for
   each of the seven declared direct and transitive Zig URLs, and requires each
   returned content hash to equal the manifest. The resulting
   `<cache>/zig-fetch-root/zig-pkg/<hash>` directories are the only Zig package
   inputs admitted to compilation; the global cache is not treated as the
   unpacked package authority.
3. Compilation is invoked only through a wrapper which creates a fresh network
   namespace. The inner build verifies that its network namespace differs from
   the caller's and has no interface other than a down loopback device. Failure
   to create or verify the namespace is fatal. Every Meson setup uses
   `--wrap-mode=nofallback`; River uses
   `zig build --system <cache>/zig-fetch-root/zig-pkg`, whose upstream contract
   disables network access.
4. The digest-checked Meson 1.4.0 archive is extracted into the cache and every
   Meson setup, compile and install command invokes that source tree's
   `meson.py` directly. The build does not use Noble's insufficient Meson 1.3.2
   or install a Python package from the network. Builds install into one new
   private prefix with `bin/`, `lib/`, `include/` and `share/`. `PATH` and
   `PKG_CONFIG_PATH` put that prefix ahead of Noble for all later builds. The
   source build order is: Wayland; wayland-protocols; libdrm, libinput, pixman,
   libxkbcommon and libdisplay-info; wlroots; River.
5. Wayland builds its scanner with documentation and tests disabled;
   wayland-protocols builds with tests disabled. libinput is configured exactly
   with `-Dmtdev=true -Dlibwacom=true -Dtests=false -Dinstall-tests=false
   -Ddocumentation=false -Ddebug-gui=false -Dlua-plugins=disabled`: mtdev and
   libwacom device support remain enabled while tests, installed tests,
   documentation, the debug GUI and Lua plugins are omitted. libxkbcommon
   disables its tools, X11/Wayland utilities, docs and registry while retaining
   the core library. Dependency test/demo programs are not installed in the
   runtime tree.
6. wlroots is configured exactly with:

   ```text
   -Dauto_features=disabled
   -Dbackends=drm,libinput
   -Drenderers=gles2
   -Dallocators=gbm
   -Dsession=enabled
   -Dxwayland=enabled
   -Dexamples=false
   -Dlibliftoff=disabled
   -Dcolor-management=disabled
   -Dxcb-errors=disabled
   ```

7. River installs with:

   ```text
   zig build --system <cache>/zig-fetch-root/zig-pkg \
     -Doptimize=ReleaseSafe -Dcpu=baseline \
     -Dxwayland -Dman-pages=false --prefix <private-prefix> install
   ```

8. After installation, each private shared object receives runpath `$ORIGIN`
   and the River executable receives `$ORIGIN/../lib`. No file is copied into a
   system library directory and the probe never invokes a system linker-cache
   update.

## Verification obligations (L5)

| # | Given / When / Then | Evidence |
|---|---|---|
| U1 | Given the selected manifest and mutation fixtures, when validation runs, then the exact source/Zig inventory passes while missing, extra, duplicate, malformed-digest, wrong-lock and malformed-Zig-hash cases fail before acquisition. | `packaging/river/test-closure-manifest.sh` |
| U2 | Given a freshly acquired cache, when the Ubuntu 24.04 probe runs, then every archive is rehashed before extraction and compilation completes in a different network namespace with no usable network; unavailable isolation or a Meson fallback is fatal, never skipped. | `.github/workflows/distro.yml` — `ubuntu-river-closure`; CI log namespace and build assertions |
| U3 | Given the built wlroots pkg-config record, when required features are queried, then `have_drm_backend`, `have_libinput_backend`, `have_gles2_renderer`, `have_gbm_allocator`, `have_session` and `have_xwayland` are all `true`; `have_x11_backend`, `have_vulkan_renderer`, `have_udmabuf_allocator` and `have_color_management` are `false`. | `packaging/river/probe-noble-closure.sh`; CI log |
| U4 | Given the staged closure and `LD_LIBRARY_PATH` unset, when River and every regular private shared object are recursively inspected through ELF metadata and dependency resolution, then each object has only its specified relative private runpath, the complete transitive dependency graph has no unresolved entry, every required private wlroots/Wayland/libdrm/libinput/pixman/xkbcommon/libdisplay-info dependency resolves below the closure, and `river -version` has exact stdout `0.4.8 +xwayland`. | `packaging/river/probe-noble-closure.sh`; CI log |
| U5 | Given the successful CI-only probe, when support claims are inspected, then it is described only as source-build and version-path evidence; Debian integration, clean install, graphical login and hardware DRM remain unverified. | SPEC 0026 boundary; later documentation changes must preserve this evidence level |

## Failure modes

- A missing or mismatched input fails before extraction; the build does not
  substitute a live URL, Noble's older library or a Meson subproject fallback.
- A missing native dependency fails configuration inside the isolated build;
  the job does not retry with networking or drop a required feature.
- A successful compile with any required feature false fails the probe.
- A private-family dependency resolving outside the closure, an
  absolute/non-private runpath, an unresolved transitive ELF dependency or a
  non-empty `LD_LIBRARY_PATH` fails before the version command.
- Version stdout other than exact `0.4.8 +xwayland` fails; the `+xwayland`
  suffix is the upstream v0.4.8 build-option signal required by this profile.
- A hosted runner without network-namespace support fails the job. There is no
  reduced headless or non-isolated success path.

## Open questions

None for this increment. Debian ownership, final filesystem paths and installed
runtime dependency metadata are deliberately deferred until this build proof is
green; they cannot be inferred from a CI prefix. A graphical claim additionally
requires an Ubuntu VM with DRM/input devices and an active logind or seatd.
