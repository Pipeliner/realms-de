# Live desktop capture review

Captured by [the successful reference VM job](https://github.com/Pipeliner/realms-de/actions/runs/34731200211/job/103654189144)
on 2026-09-13, from GitHub's tested PR merge revision
`20ef533c6989a5712598b245b937d5d87fa607ec` (PR #225 head
`bd0aeeef46c57b110c31e0a093dc49d6d09ea0d8`).

Both PNGs were downloaded unchanged and visually inspected by Codex. They show
three real foot windows managed by the installed Realm daemon, the top bar,
the which-key strip, and the grimoire opened through a real keybinding.
Sample terminals contain no personal data. Their default terminal appearance
is not evidence of completed application theming. Numeric orbit labels and
ASCII glyph fallbacks are visible; this is not proof of the final font set.

The capture environment is NixOS 26.11test, River 0.4.8 with Xwayland, in QEMU.
**Both actual PNGs are 1280×800.** The original
[capture provenance](capture-provenance.json) is preserved verbatim, including
its incorrect `1920x1080` environment suffix: that was the requested VM
resolution, not the framebuffer resolution. PNG IHDR dimensions were checked
directly; no image was resized or retouched. Future capture metadata derives
dimensions from the image instead of assuming the requested mode.

SHA-256 verification matched the original provenance:

- [Tiled desktop](realm-tiled-desktop.png): `7a538588a797829871e7e55d8e397eabf7232b979928cadf0a047d30085de695`
- [Grimoire](realm-grimoire.png): `60777f7287f64063805a70ddd4d82d251f86021075c374deb3115808705132b0`

The paired [tiled state](control-tiled-state.json) and
[grimoire state](control-grimoire-state.json) each report three managed windows.
Which-key is enabled in both; grimoire changes from false to true. State
snapshots precede capture, so the independently ticking clock can advance.
The VM test also verifies display-manager login, installed binary paths,
control replies, and successful session exit after Quit.

These are real VM screenshots, not concept art or physical-hardware evidence.
They do not establish native-package installation, portal functionality,
complete theming, or overall MVP readiness.
