# SPEC 0029 — Native graphical-session VM evidence

- **Status:** Accepted (2026-09-13)
- **Milestone:** M3
- **Issues:** [#75](https://github.com/Pipeliner/realms-de/issues/75),
  [#76](https://github.com/Pipeliner/realms-de/issues/76),
  [#98](https://github.com/Pipeliner/realms-de/issues/98)
- **Depends on:** [SPEC 0005](0005-session-startup.md),
  [SPEC 0009](0009-fedora-44-pre-alpha-baseline.md), and
  [SPEC 0028](0028-ubuntu-private-river-debian-package.md)
- **Supersedes / Superseded by:** Extends the native runtime-evidence boundary
  of SPEC 0009 and SPEC 0028; it does not replace NixOS reference-VM or
  hardware acceptance

## Purpose

Prove that the exact native packages already emitted for one commit can be
installed through normal Ubuntu 24.04 and Fedora 44 package resolution and
selected by a real display manager to start a live Realm Wayland session whose
owned health checks pass. This closes the gap between an installroot command
probe and a graphical login without rebuilding packages inside the VM or
presenting emulated devices as hardware evidence. It does not establish an
interactive application workflow merely because `GetState` or a framebuffer
exists.

## Scope

**In:** one x86_64 KVM job with Ubuntu 24.04 and Fedora 44 matrix entries; exact
same-workflow Realm package artifacts; Ubuntu's exact same-workflow private
`realm-river` artifact; Fedora's repository-owned River; checksum-pinned
official cloud images; SDDM as a test-only session-discovery/autologin fixture;
virtio graphics/input; logind and systemd-user session facts; Realm service,
control-socket, doctor and framebuffer evidence; and bounded failure cleanup.

**Out:** rebuilding any Realm or River package in the VM job; publishing a
repository or image; selecting SDDM as a Realm dependency or supported display
manager; non-x86_64 architectures; physical DRM/input, suspend, lid, or
screen-share evidence; upgrade/rollback; a week-long daily-driver claim; and
SELinux policy or AVC acceptance. Fedora keeps the image's normal enforcing
configuration unchanged, and the probe records the observed mode only. It does
not install a Realm policy, inspect AVCs, or make security-hardening a gate;
that remains post-MVP issue #106.

## Image and artifact authority

The two immutable image inputs are:

| Guest | Official image | SHA-256 |
|---|---|---|
| Ubuntu 24.04 x86_64 | `https://cloud-images.ubuntu.com/releases/noble/release-20260911/ubuntu-24.04-server-cloudimg-amd64.img` | `612b2c0cc1bc413a6cb8c38fd611794caf0f2b436c50013d8b3794db12ad7354` |
| Fedora 44 x86_64 | `https://download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2` | `28680fe5b371a5a82ebf43a31926e086a168e59949d03969c5093e7071f90b7f` |

The workflow downloads each image into a new path and verifies the literal
digest before `qemu-img` or QEMU opens it. A cache may retain bytes under a key
that includes the digest, but cache hits are rehashed. Redirects may select an
official mirror; the declared URL and expected bytes remain fixed.

The existing native package jobs upload only the exact output they already
clean-install. Artifact names include the checked-out commit SHA. The VM matrix
is in the same workflow run, declares both package producers in `needs`,
downloads the target's exact artifact name, and rejects a missing or additional
package. Ubuntu receives exactly one `realm` deb and one `realm-river` deb;
Fedora receives exactly one RPM whose queried name is `realm`. The VM job never
runs Cargo, `dpkg-buildpackage`, `rpmbuild`, or a Realm source-kit producer.
Debian package identity is read with an explicit output format whose three
unlabelled records are package, version, and architecture; the default
multi-field `dpkg-deb` display is not an identity protocol. The admitted values
remain exactly `realm`/`0.1.0`/`amd64` and
`realm-river`/`0.4.8-1`/`amd64`.

## Host KVM admission

The VM harness and QEMU run as the unprivileged GitHub Actions runner. On an
ephemeral runner where `/dev/kvm` is a character device but that runner lacks
read/write access, the workflow may grant `rw` only to the invoking runner UID
with a POSIX ACL on that exact device before invoking the harness. The workflow
must install the ACL tool explicitly. It must not use `chmod` or `chown`,
broaden access beyond the runner ACL, invoke the harness or QEMU as root, fall
back to TCG, or skip the matrix entry. A missing device, an ACL failure, or
read/write access that remains unavailable is a hard failure under the same
bounded harness contract.

## Guest contract

1. Cloud-init creates unprivileged user `alice` with an ephemeral CI SSH key.
   Host-only QEMU user networking exposes SSH; a bounded host loop fails early
   if QEMU exits. The NoCloud seed supplies a stable per-target instance ID but
   does not request a cosmetic hostname. `cloud-init status --wait` must exit
   zero; a recoverable-error status is not accepted merely because
   initialization reached `done`.
2. The guest uses its ordinary enabled distribution repositories to install
   the downloaded local Realm package(s), their dependencies, and SDDM. No
   dependency suppression is allowed. SDDM is configured only inside the
   disposable guest to autologin `alice` into the installed `realm.desktop`.
   The host stages the three per-run probe inputs in the exact
   `alice`-owned, mode-`0700` directory `/var/tmp/realm-native-vm` before the
   install. That directory must retain those exact inputs across the required
   reboot; `/tmp` is not an admissible home for post-reboot probe code. Package
   inputs may remain under `/tmp/realm-native-packages` because they are
   consumed before reboot.
3. After reboot, the probe selects `alice`'s non-remote logind session and
   requires `Type=wayland`. The installed session entry must name Realm and
   execute the installed `/usr/bin/realm-session`; no copied checkout session
   file or hand-started compositor may satisfy the test.
4. The process behind the running compositor is read through `/proc/<pid>/exe`.
   Ubuntu must resolve `/usr/lib/realm/bin/river`; Fedora must resolve
   `/usr/bin/river`. The running `realm-wm` resolves `/usr/bin/realm-wm`.
5. Through `alice`'s actual runtime directory and user bus,
   `realm-session.target`, `realm-wm.service`, and `realm-bar.service` are
   active. A newline-framed protocol-v2 client completes `Hello` then
   `GetState` against that session's production `ctl.sock`; a merely present
   socket is insufficient.
6. Installed `realmctl doctor --json` completes under the graphical session
   deadline with no failed checks. Its 32 IDs remain ordered and the exact
   accepted skips are `units/idle-lock` and `portal/filechooser`.
   `tools/floors` is the one exact package-boundary warning: its summary names
   `yazi` and `starship` as not found while reporting an observed, non-missing
   `btop` version. The finite warning set admitted by this fixture is
   `session/degraded`, `wm/capabilities`, `env/xwayland`, `palette/lint`,
   `theme/outputs`, `fonts/glyphs`, `fonts/attribution`, and `tools/floors`,
   matching SPEC 0006's non-fatal outcomes; a warning on any other row is an
   error. Any other missing reused tool, an arbitrary additional skip, or a
   failed check rejects this proof. `session/socket`,
   `session/protocol-version`, `wm/attached`, `wm/layer-shell`,
   `portal/answers`, `portal/config`, and `portal/screencast` are `ok`.
   When native packages deliver Yazi and Starship and `tools/floors` therefore
   changes to `skip`, this exact package-boundary contract must be updated; the
   fixture must not silently accept either state.
7. The host captures the emulated framebuffer plus doctor, state, logind,
   process, package, systemd-user and journal evidence before bounded shutdown.
   Unit and control readiness may precede the compositor's first painted frame,
   so a merely non-empty screendump is insufficient. For at most 15 seconds in
   total, the host captures and validates successive QEMU P6 screendumps while
   QEMU remains alive. A valid frame has an exact 8-bit RGB payload, is at least
   128 pixels high, and has non-black pixels in at least one eighth of both its
   top 64-row band and its bottom 64-row band. This observable boundary rejects
   an unpainted framebuffer containing only the QEMU pointer while admitting
   the Realm bar and key strip seen in both native guests. The last frame and
   its validation diagnostic are retained when the deadline expires. This is
   visible-session evidence, not OCR or an interactive-application claim.
   Fedora additionally records `getenforce`; its value is diagnostic evidence,
   not a security gate.

## Acceptance criteria

| # | Given / When / Then | Evidence |
|---|---|---|
| N1 | Given either declared image, when one byte or the expected digest changes, then admission fails before the overlay or VM is created | pure image-manifest fixture plus VM download log |
| N2 | Given same-run producer artifacts, when an artifact is absent, duplicated, renamed, from another commit, or has the wrong package identity, then the VM fails before guest installation; the VM workflow contains no build command | pure artifact-inventory fixture plus workflow review |
| N3 | Given the Ubuntu artifact pair and pinned Ubuntu image, when SDDM autologins `alice`, then logind reports a non-remote Wayland session from the installed Realm entry, `/proc` identifies the private River and installed Realm WM, all three Realm user units are active, `GetState` succeeds, doctor meets §Guest contract 6, the framebuffer meets §Guest contract 7, and evidence is retained | `native-session-vm (ubuntu-24.04-x86_64)` |
| N4 | Given the Fedora RPM and pinned Fedora image, when SDDM autologins `alice`, then the same session/control/doctor/framebuffer assertions pass with Fedora's `/usr/bin/river`; the evidence records the unchanged SELinux mode without inspecting AVCs or claiming policy compatibility | `native-session-vm (fedora-44-x86_64)` |
| N5 | Given a runner without `/dev/kvm`, with `/dev/kvm` still inaccessible after the narrowly scoped runner-UID ACL, a dead QEMU process, unreachable SSH, failed login, probe inputs lost across reboot, incomplete probe, or a framebuffer that remains unpainted through the one frame deadline, then the matrix entry fails rather than elevating QEMU, falling back to TCG, skipping, or reporting reduced evidence; cleanup remains bounded, the last frame is retained, and diagnostics upload on failure | VM harness reboot-retention, frame-readiness, and timeout/failure fixtures, workflow KVM-admission fixture, and artifact step with `if: always()` |

## Evidence boundary

Passing N3 or N4 proves only the named x86_64 cloud-image login path, the exact
artifacts from that workflow run, emulated virtio devices and the assertions
above. The exact `tools/floors` warning records the current native-package gap;
it does not prove charon or thoth usable and does not waive SPEC 0024's private
Yazi/Starship delivery. It does not close #78's full-week human use, #79's
unresolved lid policy, #106's SELinux/security-hardening review, #107's direct
X11/activation boundary, or any hardware-only row in SPEC 0005.

## Open questions

None for this bounded CI increment. The session's lid behavior remains an
explicit human decision in SPEC 0005 and is not inferred here.
