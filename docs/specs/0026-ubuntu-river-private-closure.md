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

The selected versions match nixpkgs revision
`9fbb54b33e91ee4ca368e35a78e0613c720600b3`, which is fixed by the repository's
`flake.lock`. The CI acquisition manifest records the URLs and raw archive
SHA-256 values below. Nix's normalized source hashes remain corroborating
recipe evidence; they do not replace the raw archive hashes consumed by this
probe.

| Input | Version / identity | Upstream archive | SHA-256 | Role |
|---|---|---|---|---|
| Zig x86_64 Linux | 0.16.0 | `https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz` | `70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00` | acquisition and River build only |
| Wayland | 1.26.0 | `https://gitlab.freedesktop.org/wayland/wayland/-/releases/1.26.0/downloads/wayland-1.26.0.tar.xz` | `64176eaa46e4969903e286f8e5ef8331affc17fdf03ac9b58381d2b23162b7a3` | private runtime and `wayland-scanner` |
| wayland-protocols | 1.49 | `https://gitlab.freedesktop.org/wayland/wayland-protocols/-/releases/1.49/downloads/wayland-protocols-1.49.tar.xz` | `ec4c8f74942d6dff7ace8b4ce4764f0ef9ff618a935d974ea77edee2ad240b14` | build-time protocol data |
| libdrm | 2.4.134 | `https://dri.freedesktop.org/libdrm/libdrm-2.4.134.tar.xz` | `ac5e74d157830eb8bee44c6a6bf3ad49774ef0dd2a72bdad74a8f20308b52a95` | private runtime |
| pixman | 0.46.4 | `https://cairographics.org/releases/pixman-0.46.4.tar.gz` | `d09c44ebc3bd5bee7021c79f922fe8fb2fb57f7320f55e97ff9914d2346a591c` | private runtime |
| libxkbcommon | 1.13.2 | `https://github.com/xkbcommon/libxkbcommon/archive/refs/tags/xkbcommon-1.13.2.tar.gz` | `acc4d5f7c3cbba5f9f8d08d8bdbeede84ecede46792f47929aa9321873385528` | private runtime; satisfies River's stricter 1.12 floor |
| libdisplay-info | 0.4.0 | `https://gitlab.freedesktop.org/emersion/libdisplay-info/-/archive/0.4.0/libdisplay-info-0.4.0.tar.gz` | `787b58aec473830b0030251ba2b880560b4b3853dc5d6961a6e9701abae29b55` | private DRM runtime |
| wlroots | 0.20.2 | `https://gitlab.freedesktop.org/wlroots/wlroots/-/archive/0.20.2/wlroots-0.20.2.tar.gz` | `972c7ac44b17828f4702bfae7cd8347346a3fb5b2c1076cfa2c3fcedac5ec343` | private compositor runtime |
| River | tag `v0.4.8`, commit `c4b5f706314555f4846e25b8d3635631387b3fdd` | `https://codeberg.org/river/river/releases/download/v0.4.8/river-0.4.8.tar.gz` | `6d4030526e307e40de357167b4d6daacb583aed353dd93e32e1314c2d34400fa` | selected compositor |

River's exact Zig package-manager inputs come from the selected release's
`build.zig.zon` and are also explicit manifest records:

| Package | Acquisition URL | Zig content hash |
|---|---|---|
| pixman | `https://codeberg.org/ifreund/zig-pixman/archive/v0.3.0.tar.gz` | `pixman-0.3.0-LClMnz2VAAAs7QSCGwLimV5VUYx0JFnX5xWU6HwtMuDX` |
| wayland | `https://codeberg.org/ifreund/zig-wayland/archive/v0.6.0.tar.gz` | `wayland-0.6.0-lQa1kqz8AQADQmdNJsNhLoNHcnEGEUjrOaPV-dtEnEmX` |
| wlroots | `https://codeberg.org/ifreund/zig-wlroots/archive/v0.20.1.tar.gz` | `wlroots-0.20.1-jmOlcqNVBAB3uB5oqBTzpRlwu-FmMyyZMVAWCe5kmcSt` |
| xkbcommon | `https://codeberg.org/ifreund/zig-xkbcommon/archive/v0.4.0.tar.gz` | `xkbcommon-0.4.0-VDqIe0i2AgDRsok2GpMFYJ8SVhQS10_PI2M_CnHXsJJZ` |
| translate-c | `git+https://codeberg.org/ziglang/translate-c/#57c559cf581b1fcad90494eda219f98abeb155ce` | `translate_c-0.0.0-Q_BUWlX1BgCD1wo6uo97prlp9VJ4gxAjwN_vZ7nsSjGN` |

The validator rejects an absent or extra selected input, duplicate identity,
an archive URL that is not HTTPS, a Zig dependency URL that is neither HTTPS
nor `git+https`, a non-lowercase 64-digit archive digest, a malformed Zig
content hash, or a manifest nixpkgs revision different from the lock. Archive
bytes are hashed again before extraction and before network-disabled
compilation.

## Noble system boundary

Noble supplies dependencies whose versions already meet wlroots 0.20.2's
requirements: Meson 1.3.2, libinput 1.25.0, libseat 0.8.0, Mesa EGL/GLES/GBM,
XCB/Xfixes 1.15 and Xwayland 23.2.6. The CI job also installs their development
closure: build-essential, pkg-config, ninja-build, Python, bison, patchelf,
libffi, expat, libpciaccess, pthread stubs, udev, libevdev, libcap, GL/EGL/GLES,
GBM, X11, the XCB composite/EWMH/ICCCM/render/res/xfixes packages, hwdata,
xkb-data and Xwayland.

The private source set is required because Noble remains below these floors:

| Dependency | Required by selected source | Noble |
|---|---:|---:|
| Wayland | 1.24.0 | 1.22.0 |
| wayland-protocols | 1.47 | 1.45 |
| libdrm | 2.4.129 | 2.4.125 |
| pixman | 0.43.0 | 0.42.2 |
| libxkbcommon | 1.12.0 (River) | 1.6.0 |
| libdisplay-info | 0.2.0 | 0.1.1 |
| wlroots | 0.20.x | 0.17.1 |

## Acquisition and build contract (L4)

1. The acquisition command validates the checked-in manifest, creates a new
   empty cache, downloads every archive to its canonical manifest filename,
   and verifies its SHA-256 before it may be used. It extracts the selected Zig
   binary only after its checksum passes.
2. Acquisition runs `zig fetch --global-cache-dir <cache>` once for each of the
   five declared Zig URLs and requires the returned content hash to equal the
   manifest. The resulting `<cache>/p/<hash>` directories are the only Zig
   package inputs admitted to compilation.
3. Compilation is invoked only through a wrapper which creates a fresh network
   namespace. The inner build verifies that its network namespace differs from
   the caller's and has no interface other than a down loopback device. Failure
   to create or verify the namespace is fatal. Every Meson setup uses
   `--wrap-mode=nofallback`; River uses `zig build --system <cache>/p`, whose
   upstream contract disables network access.
4. Builds install into one new private prefix with `bin/`, `lib/`, `include/`
   and `share/`. `PATH` and `PKG_CONFIG_PATH` put that prefix ahead of Noble for
   all later builds. The order is:
   Wayland; wayland-protocols; libdrm, pixman, libxkbcommon and libdisplay-info;
   wlroots; River.
5. Wayland builds its scanner with documentation and tests disabled;
   wayland-protocols builds with tests disabled. libxkbcommon disables its
   tools, X11/Wayland utilities, docs and registry while retaining the core
   library. Dependency test/demo programs are not installed in the runtime
   tree.
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
   zig build --system <cache>/p -Doptimize=ReleaseSafe -Dcpu=baseline \
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
| U4 | Given the staged closure, when ELF metadata and dependencies are inspected, then River has only the relative private runpath, every required private wlroots/Wayland/libdrm/pixman/xkbcommon/libdisplay-info dependency resolves below the closure, no dependency is missing, and the executable prints `river 0.4.8` for `-version`. | `packaging/river/probe-noble-closure.sh`; CI log |
| U5 | Given the successful CI-only probe, when support claims are inspected, then it is described only as source-build and version-path evidence; Debian integration, clean install, graphical login and hardware DRM remain unverified. | SPEC 0026 boundary; later documentation changes must preserve this evidence level |

## Failure modes

- A missing or mismatched input fails before extraction; the build does not
  substitute a live URL, Noble's older library or a Meson subproject fallback.
- A missing native dependency fails configuration inside the isolated build;
  the job does not retry with networking or drop a required feature.
- A successful compile with any required feature false fails the probe.
- A private dependency resolving outside the closure, an absolute runpath or an
  unresolved ELF dependency fails before the version command.
- A hosted runner without network-namespace support fails the job. There is no
  reduced headless or non-isolated success path.

## Open questions

None for this increment. Debian ownership, final filesystem paths and installed
runtime dependency metadata are deliberately deferred until this build proof is
green; they cannot be inferred from a CI prefix. A graphical claim additionally
requires an Ubuntu VM with DRM/input devices and an active logind or seatd.
