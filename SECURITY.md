# Security

helm is **pre-alpha**. There is no release, no packages, and nothing you can log
into. That changes what a useful security report looks like right now, but not
whether we want one.

## What is security-relevant here

A desktop environment sits between the user and everything they run, so parts of
it are security surfaces even at this stage. These are the ones we already know
about and design against:

| Surface | Why it matters | Where it lives |
|---|---|---|
| **The control socket** — `$XDG_RUNTIME_DIR/helm/ctl.sock` (no `/tmp` fallback or production override) | Anything that can write to it can spawn processes, move windows and change the session's state. The server therefore uses runtime/socket modes, refuses unsafe path entries, admits only same-euid `SO_PEERCRED` peers, requires Hello before requests, and bounds every connection. | [`helm-core::ipc`](crates/helm-core/src/ipc.rs), and `helm-session` when it exists |
| **Generated theme files** | `helm ctl theme apply` renders templates and writes them into the user's config — `gtk.css`, Kvantum, terminal, `yazi`, `btop`, `starship`. A template that can be made to escape its output, or a write that can be redirected, writes attacker-chosen content into files other programs read. | `helm-theme` (planned, M1), `configs/templates/` |
| **The launcher** | Draft SPEC 0013's verified path accepts a typed desktop id, pins an immutable desktop/ELF/cwd/generation plan, never uses a shell, and refuses D-Bus activation before launch-local effects. Raw `Spawn(argv)`, direct fuzzel application mode and future spells are separate unverified admission classes and must never report themed/verified status. | fuzzel picker UI plus Helm broker as the stopgap (M3), `helm-hecate` (M4) |
| **Session startup** | Session entry sets and exports the environment into systemd and D-Bus. Getting this wrong leaks variables into services that should not have them, or lets a stale value point a portal somewhere unexpected. | ADR 0011, `packaging/` (planned, M3) |
| **The compositor and the session daemon** | They hold input and window state for the whole session. Once `helm-compositor` exists it also mediates screen capture and clipboard. | `helm-session` (M2), `helm-compositor` (M5) |

## What is not yet in scope

Not because these do not matter, but because claiming coverage we do not have
would be worse than saying so:

- **There is no lock screen.** Which one to ship is an open question on the
  [front page](README.md#needs-a-human). Until it is answered, helm does not
  lock, and must not be relied on to.
- **Screen capture and clipboard mediation** are the portal's job today. helm
  does not yet add a permission model of its own.
- **Sandboxing and per-app permissions** are out of scope for v1.
- **No release has been signed**, because there is no release. Package signing
  is part of the open packaging question.
- **Multi-user and remote scenarios** are untested. helm assumes one local user
  on one seat.

## Reporting something

If you believe you have found a vulnerability, please **do not open a public
issue**.

1. Use GitHub's private reporting:
   **[Report a vulnerability](https://github.com/Pipeliner/realms-de/security/advisories/new)**
   (Security → Advisories on the repository). This creates a private advisory
   visible only to maintainers.
2. If private reporting is unavailable, email
   [`vadim.evard@gmail.com`](mailto:vadim.evard@gmail.com). Do not disclose
   vulnerability details in a public issue.

Please include, as far as you can: what you did, what happened, what you
expected, the distribution and compositor, and whether `helm ctl doctor` had
been run. A reproducer is worth more than a description.

**What to expect.** This is a small, volunteer, pre-alpha project. We aim to
acknowledge a report within a week and to say plainly what we intend to do
about it. We will not ask you to keep a finding quiet indefinitely; if we cannot
fix something we will say so and document it as a known limitation.

There is no bug bounty.

## Supported versions

None yet. There has been no release, so there is nothing to backport to. Until
M3, `main` is the only thing that exists and the only thing that gets fixed.

| Version | Supported |
|---|---|
| `main` | Yes, on a best-effort basis |
| any tag | There are none |
