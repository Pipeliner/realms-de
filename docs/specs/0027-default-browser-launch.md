# SPEC 0027 — Default browser launch

- **Status:** Accepted (2026-09-13, delegated technical decision for MVP #232)
- **Scope:** MVP capability 5; default Nav `b` / hermes binding

## Behavior

The shipped `realm-browser` command takes no arguments. It queries
`xdg-settings get default-web-browser` and launches that desktop application
with `gtk-launch`, without a URL or command-line shell evaluation. The user’s
chosen browser and its normal startup behavior remain authoritative. Realm
does not set a default, select a brand, or open an external website.

The query must succeed and return exactly one nonempty desktop-file ID ending
in `.desktop`, with no path separators, whitespace, or leading dash. An absent
or invalid default is a visible error on stderr and a nonzero exit. Missing
helpers and desktop launch failures likewise produce a nonzero result; there
is no guessed fallback browser. The desktop ID is passed as one argument.

The existing binding remains ordinary `Spawn(["realm-browser"])`. The existing
process worker observes and reaps this helper. A successful helper exit means
the desktop launcher accepted the request, not that a window was proven open.
Desktop activation may reuse an existing process; this helper must not claim
to own that process or attach a theme-generation lifetime to its own short
life. Existing session environment is inherited unchanged. Full application
theme activation remains governed by its separate accepted contract.

## Packaging and verification

Nix, Debian, and Fedora install the helper and provide `xdg-settings` and
`gtk-launch` on its execution path. A user-selected installed browser is a
documented prerequisite; no browser brand is mandatory.

Tests first reproduce the absent helper, then cover exact query/launch argv,
no default, malformed IDs, query failure, launch failure, and rejection of
arguments. Fake commands test dispatch, not browser usability.

The installed graphical VM additionally configures a real browser desktop
entry, presses the actual default binding, and observes its managed window.
Only that runtime evidence satisfies browser-launch acceptance. Native package
build and installation verification runs in CI, not locally.

The desktop launcher resolves desktop entries using the platform implementation
rather than a new Realm desktop-file parser. GTK documents this launch model:
https://docs.gtk.org/gtk4/gtk4-launch.html (the installed GTK 3 helper is
`gtk-launch`; packaging must verify its actual availability).
