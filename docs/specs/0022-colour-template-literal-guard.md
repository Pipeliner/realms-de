# SPEC 0022 — Colour-template literal guard

- **Status:** Accepted (2026-08-31)
- **Milestone:** M1
- **Issue:** [#24](https://github.com/Pipeliner/realms-de/issues/24)
- **Decisions:** [ADR 0005](../adr/0005-palette-toml-single-source.md), [SPEC 0002](0002-theme-pipeline.md)
- **Supersedes / Superseded by:** —

## Purpose

`palette.toml` is the sole source of Helm's themed colour values. Generated
output necessarily contains target-format colour tokens, so this contract
guards the **shipped template sources** before rendering. It refuses a
hard-coded colour literal while allowing the target-format framing around a
Helm placeholder.

## Scope

**In:** the eight source files currently embedded by `helm_theme::templates()`:
`gtk3.css`, `gtk4.css`, `foot.ini`, `yazi-theme.toml`, `btop.theme`,
`starship.toml`, `fuzzel.ini`, and `qt6ct-colors.conf`; exact diagnostics;
hostile fixtures; and the palette CI workflow.

**Out:** colours in rendered generations; package/source extraction; a general
CSS parser or named-colour table; user configuration; and changing palette
values, template vocabulary, publication, or activation semantics. Those are
separate contracts.

The existing repository-wide `#RRGGBB` check remains in force. This guard is
an additional target-aware check; it does not weaken existing exclusions or
allowlists.

## Behaviour

1. One checked-in checker receives a repository root, scans the complete fixed
   template inventory, and exits nonzero on a violation. The inventory is
   exactly the eight paths named above and must equal the set of literal
   `::core::include_str!("../../../configs/templates/<path>")` operands in
   `crates/helm-theme/src/template.rs`. A missing expected file, duplicate or
   additional template operand, additional file under `configs/templates/`, or
   a template without a declared target grammar is a refusal. Adding or
   removing a shipped template requires an accepted amendment and fixture
   update. The one top-level `pub fn templates()` definition is itself the
   catalogue: its body must be a direct tail `vec![Template { ... }, ...]`
   expression containing only the eight direct `Template` records. Nested
   functions, conditionally disabled records, helper-return indirection, and
   records outside that direct vector are not catalogue evidence and cause a
   refusal. The source guard is lexical only: a compiled `helm_theme` unit
   test separately proves the actual `templates()` id-to-source-byte mapping
   against the eight qualified built-in includes, so macro expansion cannot
   substitute different compiled sources while preserving lexical text.
2. Every diagnostic names the repository-relative path, one-based line, and
   the complete offending token or value. The palette workflow invokes this
   checker, and the fixture test invokes the same entry point.
3. In GTK CSS templates, the checker scans all source bytes outside complete
   Helm `{{ ... }}` placeholders, including comments and quoted strings. A
   `#` followed by one or more contiguous ASCII hexadecimal digits is one
   candidate: exactly 3, 4, 6, or 8 digits is a literal-colour failure; every
   other digit count is a malformed-colour failure. A candidate followed by an
   ASCII hexadecimal digit is one complete candidate, never a valid prefix.
   An ASCII-case-insensitive `rgb` or `rgba` identifier with no preceding or
   following ASCII identifier byte (`A-Z`, `a-z`, `0-9`, `_`, or `-`), followed
   by optional CSS whitespace and `(`, is a literal-colour failure through its
   matching `)` across the complete file; nested parentheses are counted and
   an unclosed `(` is a malformed-colour failure through end of file. This
   lexical rule covers
   integer, decimal, percentage, comma, whitespace, slash, CRLF and form-feed
   spellings without claiming to parse CSS or named colours. Placeholder
   expressions such as `{{ border.seam.rgba(border.seam_alpha) }}` remain valid
   source syntax.
4. In `foot.ini`, only colour-key values in the `[colors]` section are colour
   positions. The one non-colour setting is exactly `alpha=1.0`; another key or
   alpha spelling fails. Each colour-key value is either one or two
   whitespace-separated complete Helm placeholder expressions ending in
   `.bare`, or it fails. A raw contiguous six- or eight-ASCII-hex-digit
   candidate, a placeholder without `.bare`, and extra non-whitespace bytes
   fail. Other Foot sections and numeric settings are not colour positions.
5. In `fuzzel.ini`, every value in `[colors]` is exactly one complete Helm
   placeholder expression ending in `.bare`, immediately followed by lowercase
   ASCII `ff`, with optional surrounding whitespace only. A raw `RRGGBBAA`, a
   raw `RRGGBB` plus `ff`, another alpha prefix/suffix, a placeholder without
   `.bare`, and extra bytes fail. The current renderer defines `.bare` as the
   only admitted six-bare-hex source form; this contract adds no new transform.
6. In `qt6ct-colors.conf`, `[ColorScheme]` contains exactly the three keys
   `active_colors`, `disabled_colors`, and `inactive_colors`; another key ending
   in `_colors` fails as unclassified. Each comma-separated field is nonempty,
   has no empty/trailing entry, and each value is exactly `#ff` followed by one
   complete Helm placeholder expression ending in `.bare`, with optional edge
   whitespace only. A raw `#AARRGGBB`, changed alpha prefix, incomplete
   placeholder, or extra non-whitespace byte fails.
7. In `btop.theme`, each `theme[NAME]` value must be exactly one quoted complete
   Helm placeholder expression with no literal bytes inside the quotes. In
   `yazi-theme.toml`, each inline-table `fg` or `bg` value must be exactly one
   quoted complete Helm placeholder expression, except the existing literal
   `"reset"`. In `starship.toml`, each `fg:` segment in a quoted style value
   must be immediately followed by exactly one complete Helm placeholder
   expression. In all three formats, any raw `#` hexadecimal candidate or
   target-position extra bytes fail; candidates use the same complete-span and
   malformed-length rule as CSS.
8. CRLF, a second literal, and every comment/string selected by the rules above
   are checked. Fixture-only files are never production inputs and may contain
   hostile literals.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the exact shipped eight-template inventory and its literal Rust catalogue operands, when the checker runs, then it passes and detects a missing, additional, unclassified, catalogue-divergent, comment-spoofed, raw-string-spoofed, nested-function-spoofed, disabled-top-level-function-spoofed, or disabled-inner-decoy template source; the compiled mapping test also rejects macro-generated source substitution. | `docs/test-colour-template-literals.sh` — `shipped`, `missing-template`, `additional-template`, `catalogue-duplicate-operand`, `catalogue-redirected-source`, `catalogue-comment-spoof`, `catalogue-raw-string-spoof`, `catalogue-raw-byte-string-spoof`, `catalogue-raw-c-string-spoof`, `catalogue-nested-function-spoof`, `catalogue-cfg-disabled-top-level-spoof`, `catalogue-disabled-inner-decoy-spoof`; `template::tests::compiled_catalogue_embeds_the_declared_template_sources` |
| A2 | Given a GTK template with literal or malformed hex, or case-insensitive boundary-valid `rgb`/`rgba` outside a placeholder across CSS lexical spellings and lines, when the checker runs, then it fails with path, line, and complete token; a placeholder transform passes. | `docs/test-colour-template-literals.sh` — `gtk-rgb-across-crlf`, `gtk-malformed-hex` |
| A3 | Given a Foot `[colors]` literal or invalid `alpha` setting, a Fuzzel raw/malformed opacity composition, and a Qt raw/incorrect-alpha/empty/unclassified colour field, when the checker runs, then each fails; the shipped placeholder compositions pass. | `docs/test-colour-template-literals.sh` — `foot-invalid-alpha`, `foot-raw-colour`, `fuzzel-raw-colour`, `fuzzel-invalid-opacity`, `qt-raw-colour`, `qt-unclassified-colour-field` |
| A4 | Given btop, Yazi, or Starship with a target-position literal or malformed value outside a placeholder, including CRLF and repeated-literal fixtures, when the checker runs, then every violation is reported. | `docs/test-colour-template-literals.sh` — `btop-raw-colour`, `yazi-raw-colour`, `yazi-unquoted-colour`, `starship-literal`, `starship-non-placeholder-style` |
| A5 | Given the palette CI workflow, when it runs on a pull request, then it invokes the same checker; a hostile fixture proves a checker failure is surfaced. | `docs/test-colour-template-literals.sh` — `ci-invokes-checker` |

## Failure modes

The guard prevents “colour written down twice” from entering a shipped template.
It must not reject correct generated output, mistake the Qt `#ff` or Fuzzel
`ff` framing bytes for palette literals, or silently omit an added template.

## Open questions

None in this scope. CSS named-colour handling, package extraction, and
rendered-output provenance are explicitly deferred rather than implicitly
allowed.
