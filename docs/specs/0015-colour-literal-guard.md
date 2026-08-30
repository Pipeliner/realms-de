# SPEC 0015 — Colour-literal guard evidence

- **Status:** Draft — documentation-only candidate for #24; it authorizes no implementation.
- **Milestone:** M1
- **Decisions:** [ADR 0005](../adr/0005-palette-toml-single-source.md), [ADR 0006](../adr/0006-oklab-contrast-not-filters.md), [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Supplements:** [SPEC 0002](0002-theme-pipeline.md) A1/A4 and [SPEC 0011](0011-theme-activation-generations.md)
- **Acceptance gate:** This Draft cannot become Accepted until #24 selects the target-format grammar and exception schema below, and the checked-in package-extractor table has the required nonempty Nix, Debian, and RPM entries, without narrowing ADR 0005 by implication.
- **Supersedes / Superseded by:** —

## Purpose

ADR 0005 makes repository-root `palette.toml` the only source of themed colour values. This guard rejects a second source in shipped theme inputs, while proving at the render boundary that the colour literals which must appear in generated output originate from the captured derived palette. It does not claim that a sealed generation stores a provenance trace.

## Scope

**In:** byte-oriented scanning of defined tracked theme-source inputs; a narrow test/fixture boundary with anti-bypass checks; machine-readable exact exceptions; hostile fixtures; and a pre-publication in-memory provenance assertion.

**Out:** choosing palette values or fields; changing placeholder vocabulary or transforms; parsing third-party user configuration; generation publication, manifest format, integrity, or reload semantics (SPEC 0011); and changing ADR 0005's single-palette decision.

## Draft behaviour

### Source inventory and test/fixture boundary

The source scanner receives bytes, not decoded text. It scans **every tracked regular file** except repository-root `palette.toml`, `README.md`, the provenance/prose roots `design/**`, `docs/**`, and `.claude/**`, and the narrowly defined test/fixture regions below. It scans comments, strings, CRLF files, and all ASCII byte sequences regardless of UTF-8 validity. A NUL byte in an in-scope text input is a failure naming the path and byte offset. A nested file named `palette.toml`, and a literal in an otherwise unlisted tracked root, are in scope and fail.

Rust is excluded only within a syntactically delimited `#[cfg(test)] mod … { … }` module; production items after that module remain scanned. Integration tests and non-Rust fixtures are excluded only below `crates/*/tests/**` or `**/tests/fixtures/**`. These exclusions do not make their bytes production-consumable.

The checker has one finite, conservative production-input extractor. It resolves only: (1) the static `helm_theme::template::templates()` catalogue; (2) production Rust `include_str!` and `include_bytes!` invocations whose argument is one ordinary relative string literal; (3) `build.rs` direct `std::fs::read`, `read_to_string`, and `copy` calls whose source operand is one ordinary relative string literal; and (4) literal source operands described by the checked-in package-extractor table below. Each resolved path is normalized repository-relative. A production input may not resolve beneath an excluded test/fixture region. A use of one of those inclusion forms with a nonliteral, concatenated, environment-derived, generated, or otherwise unrecognized reference fails closed. Thus a production source cannot consume excluded test/fixture data, whether it is catalogued, embedded, built, or packaged.

The package-extractor table is exactly repository-root `configs/colour-guard-package-extractors.toml`, encoded as TOML with `version = 1` and one or more `[[form]]` entries. Each entry has exactly `id`, `lane`, `file`, and `source_operand`; unknown keys, duplicate `id`s, missing lanes, or an empty table fail closed. Before this Draft can become Accepted the table SHALL contain, at minimum, these nonempty entries:

```toml
version = 1

[[form]]
id = "nix-src-plus-install"
lane = "nix"
file = "packaging/nix/package.nix"
source_operand = "${src + STRING}; STRING is a double-quoted '/' + PATH; source operand of install -D*"

[[form]]
id = "debian-dh-install-line"
lane = "debian"
file = "packaging/debian/install"
source_operand = "first whitespace-delimited PATH field of a non-comment dh_install line"

[[form]]
id = "rpm-install-command"
lane = "rpm"
file = "packaging/fedora/helm.spec"
source_operand = "first PATH operand after install -DpmMODE in %install"
```

`PATH` is one nonempty, repository-relative, slash-separated path of ASCII bytes with no quote, whitespace, glob, `$`, `.` or `..` component. `STRING` is one double-quoted Nix string whose bytes are exactly `/` followed by `PATH`; the Nix form accepts only `${src + STRING}` as the source operand of an `install -D*` command. The Debian form accepts only its stated first field; the RPM form accepts only its stated first operand. An install/package form not represented by one of these exact table entries is unknown and fails closed, including dynamic shell expansion, a new manifest key, or a different package file. Adding a form requires an accepted amendment that updates this table and its hostile fixtures; it cannot be added as an implementation-only allowlist.

### Target-format literal grammar

The checker reports complete ASCII token byte spans with one-byte lexical boundaries: a hex token may not be immediately followed by an ASCII hex digit; a function identifier may not be immediately preceded or followed by an ASCII identifier byte (`A-Z`, `a-z`, `0-9`, `_`, `-`). A hash followed by one or more ASCII hex digits is consumed as one candidate; a candidate whose digit count is not 3, 4, 6, or 8 is an invalid colour-candidate failure over its complete span, never a six-digit prefix match. Matching is case-insensitive where the target format is CSS.

For CSS templates (`.css`), the Draft target grammar is CSS colour tokens accepted by the shipped GTK targets:

```text
hex := "#" (HEX{3} | HEX{4} | HEX{6} | HEX{8}) boundary
rgb-function := ("rgb" | "rgba") "(" css-component-list ")"
hsl-function := ("hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch" | "color") "(" balanced-css-function-bytes ")"
css-component-list := legacy comma-separated or modern whitespace/slash-separated numeric or percentage components
css-whitespace := space, tab, CR, LF, or FF
HEX := [0-9A-Fa-f]
```

The `rgb`/ `rgba` function rule accepts integer, fractional (including `.3`), and percentage components, all CSS whitespace, and legacy or modern slash form. The checker treats a CSS named-colour keyword as a colour literal only when a CSS value-token parser identifies it in a colour-valued declaration; it does not grep ordinary prose identifiers. The named-keyword set is the CSS Color keyword set supported by the GTK CSS parser used by the target lane, recorded as a versioned checker table.

For non-CSS shipped templates, the grammar is `hex` only. A raw CSS function in a non-CSS target is not a supported target-format literal and is outside this Draft's detection claim. Unsupported future target formats and syntaxes must either be added with a target-format grammar and hostile fixtures, or be called residual risk in an accepted amendment; their omission here is not permission to hardcode colours.

A Helm expression such as `{{ border.seam.rgba(border.seam_alpha) }}` is not a CSS literal: it is recognized as template syntax before CSS token scanning. The scanner SHALL reject a numeric CSS `rgba` literal beside such an expression.

### Exact machine-readable exception records

The only consumer exceptions are entries in one checked-in, versioned TOML file, `configs/colour-literal-exceptions.toml`:

```toml
version = 1

[[exception]]
path = "relative/file.ext"
byte_start = 123       # zero-based, inclusive
token = "#000000"      # exact ASCII bytes at byte_start
reason = "format-mandated value; cannot be represented by the palette"
```

The schema rejects unknown keys, duplicate `(path, byte_start)` pairs, empty reasons, non-canonical paths, absolute paths, `.` or `..` path components, non-existent paths, and tokens not matching the target-format grammar at exactly the recorded byte span. An exception covers exactly `token.len()` bytes, never a grammar or line. It may not name `palette.toml`, any prose exclusion, `configs/templates/`, an excluded test/fixture path, or a generated output path. A moved token, changed token, or second literal in the same file fails unless it has its own valid record. The record file itself is parsed as checker metadata and is not a themed source input.

### Pre-publication output provenance

The source literal ban SHALL NOT run unchanged over generated outputs: correct output contains rendered colours. For `apply` and `apply_with_snapshot`, the authoritative provenance assertion runs in memory after each template is rendered and before any `GenerationPublication` operation. For candidate rendering in `diff` and `diff_with_snapshot`, it runs before byte comparison and performs no generation mutation.

Each colour-placeholder expansion produces an immutable record containing: template ID; exact source byte span of the whole placeholder; resolved palette path; transform chain; derived-palette snapshot SHA-256; normalized output path; and exact output byte span. For every recognized output colour token, its complete output byte span SHALL be wholly equal to one, and only one, colour-placeholder expansion span. The expansion record's bytes must equal the token, and its palette path/transform must resolve from that candidate's derived palette snapshot to the same bytes. Partial coverage, overlap/composition, a forged record, a token merely equal in value to a different palette key, or no record is a failure.

The trace is ephemeral render evidence: it is discarded before `GenerationPublication`; SPEC 0011 manifests and sealed generations do not claim to carry it. After sealing, existing manifest/digest validation continues to be the integrity boundary. No checker may reconstruct or infer palette-path provenance solely from sealed output bytes.

### Shared checker and diagnostics

One checked-in checker library and one CI-facing entry point SHALL implement source scanning, the finite production-input extractor, exception validation, and pre-publication provenance assertions. CI, local tests, and hostile fixtures invoke that entry point or the same library; a second shell/Rust regex implementation is forbidden.

A source diagnostic SHALL name canonical repository path, one-based line and column computed from bytes, inclusive byte span, and complete token. It shall emit a GitHub annotation in CI. A generated diagnostic SHALL name normalized output path, template ID, output byte span, complete token, and the missing, partial, overlapping, or mismatched provenance field.

## Acceptance criteria

Test names remain blank until this Draft is accepted, tests are written and observed failing, and implementation lands.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given an in-scope template, production Rust/config/script file, comment, string, CRLF file, non-UTF-8 byte fixture, or an otherwise-unlisted tracked root containing one or two literals, when the CI-facing checker runs, then each violation reports canonical path, line, column, byte span, and complete token. | |
| A2 | Given CSS hex forms of 3/4/6/8 digits, upper/lower/mixed-case RGB/RGBA forms, integer/fractional/`.3`/percentage components, comma and slash forms, and LF/FF-separated arguments, when the CSS scanner runs, then each complete token is rejected; adjacent hex/identifier bytes are diagnosed without prefix matches. | |
| A3 | Given `{{ …rgba(...) }}` template syntax and a numeric CSS `rgba` literal in the same CSS template, when the checker runs, then only the latter is rejected. | |
| A4 | Given a Rust test module followed by production code, a literal in an excluded integration test/fixture, and a literal in production code after the test module, when the checker runs, then only the test-only literal is exempt. | |
| A5 | Given a catalogue path, literal `include_str!` and `include_bytes!`, literal build-script read/copy or generated embedding, Nix `${src + STRING}` in `install -D*`, a Debian `packaging/debian/install` first-field path, and an RPM `%install` `install -DpmMODE` first operand pointing at an excluded test/fixture path, when the finite extractor runs, then each is refused; a nonliteral/dynamic operand or an unlisted package-install form also fails closed. | |
| A6 | Given valid, duplicate, traversal, noncanonical, stale, moved-token, second-token, and forbidden-template exception records, when the checker validates the TOML schema and sources, then only the valid exact token is exempt. | |
| A7 | Given an `apply_with_snapshot` candidate rendered at default contrast and `contrast = 1.30`, when the pre-publication assertion runs, then every recognised output token has exactly one full-span record with matching template ID/source span, palette path/transform, derived snapshot SHA-256, normalized output path, and bytes. | |
| A8 | Given split hex/RGB/RGBA construction, an unexpanded output literal equal to a palette value, equal values from distinct palette paths, partial/overlapping spans, forged snapshot identity, or a mismatched output path, when provenance is asserted, then publication is refused before `GenerationPublication` and the diagnostic identifies the failed relation. | |
| A9 | Given a candidate whose pre-publication assertion passed and a subsequently sealed generation whose output bytes are tampered, when ordinary SPEC 0011 manifest/digest validation is run, then it rejects the tampering; the provenance API cannot accept the sealed bytes as a trace or infer provenance from them. | |
| A10 | Given the same hostile source, exception, and provenance fixture corpus, when CI and the local test command run, then both invoke the same checker entry point/library and produce identical pass/fail classifications and diagnostics. | |

## Budgets

The guard shall not add a new theme-apply budget. The pre-publication assertion is included in SPEC 0002's existing full-set apply budget of under 150 ms; a numeric overhead claim requires measurement before this Draft may be Accepted.

## Failure modes

This spec addresses PITFALLS.md's “colour written down twice — the two drift.” It must not mistake correct generated output for a source literal, allow a test/fixture or exception bypass, truncate a diagnostic token, or claim that a sealed generation carries provenance it does not store.

## Open questions

- The exact GTK CSS parser/version and its versioned named-colour keyword table must be recorded before acceptance.
- A future target format may require a separate accepted target-format grammar. This Draft intentionally makes no claim that its CSS/hex grammar detects every possible colour syntax outside the shipped targets.
- The checker implementation may choose its internal data structures, but cannot weaken byte spans, exact exception schema, lifecycle boundary, or acceptance rows above.
