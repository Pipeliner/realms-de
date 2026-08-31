#!/bin/sh
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
checker=$root/scripts/check-colour-template-literals
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-colour-template-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

fixture=$tmp/repo
mkdir -p "$fixture"
cp -R "$root/configs" "$fixture/configs"
mkdir -p "$fixture/crates/helm-theme/src" "$fixture/.github/workflows"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"
cp "$root/.github/workflows/palette.yml" "$fixture/.github/workflows/palette.yml"

run=0
expect_pass() {
    name=$1
    shift
    run=$((run + 1))
    if "$@" >"$tmp/$name.out" 2>"$tmp/$name.err"; then
        printf 'ok %s - %s\n' "$run" "$name"
    else
        printf 'not ok %s - %s\n' "$run" "$name" >&2
        cat "$tmp/$name.out" "$tmp/$name.err" >&2
        exit 1
    fi
}

expect_fail() {
    name=$1
    needle=$2
    shift 2
    run=$((run + 1))
    if "$@" >"$tmp/$name.out" 2>"$tmp/$name.err"; then
        printf 'not ok %s - %s (unexpected success)\n' "$run" "$name" >&2
        exit 1
    fi
    if grep -F -q "$needle" "$tmp/$name.out" "$tmp/$name.err"; then
        printf 'ok %s - %s\n' "$run" "$name"
    else
        printf 'not ok %s - %s (missing %s)\n' "$run" "$name" "$needle" >&2
        cat "$tmp/$name.out" "$tmp/$name.err" >&2
        exit 1
    fi
}

expect_pass shipped "$checker" --root "$fixture"

printf '\nextra = "#aabbcc"\n' >>"$fixture/configs/templates/starship.toml"
expect_fail starship-literal 'starship.toml' "$checker" --root "$fixture"
cp "$root/configs/templates/starship.toml" "$fixture/configs/templates/starship.toml"

sed -i 's/fg:{{ pantheon\.thoth }}/fg:red/' "$fixture/configs/templates/starship.toml"
expect_fail starship-non-placeholder-style 'starship.toml' "$checker" --root "$fixture"
cp "$root/configs/templates/starship.toml" "$fixture/configs/templates/starship.toml"

printf '\nprobe { color: RGB(1,\r\n2,\r\n3) }\n' >>"$fixture/configs/templates/gtk3.css"
expect_fail gtk-rgb-across-crlf 'gtk3.css' "$checker" --root "$fixture"
cp "$root/configs/templates/gtk3.css" "$fixture/configs/templates/gtk3.css"

printf '\nprobe { color: #12345 }\n' >>"$fixture/configs/templates/gtk3.css"
expect_fail gtk-malformed-hex 'gtk3.css' "$checker" --root "$fixture"
cp "$root/configs/templates/gtk3.css" "$fixture/configs/templates/gtk3.css"

sed -i 's/^alpha=1\.0$/alpha=0.5/' "$fixture/configs/templates/foot.ini"
expect_fail foot-invalid-alpha 'foot.ini' "$checker" --root "$fixture"
sed -i 's/^alpha=0\.5$/alpha=1.0/' "$fixture/configs/templates/foot.ini"

sed -i 's/{{ background\.void\.bare }}/112233/' "$fixture/configs/templates/foot.ini"
expect_fail foot-raw-colour 'foot.ini' "$checker" --root "$fixture"
cp "$root/configs/templates/foot.ini" "$fixture/configs/templates/foot.ini"

sed -i 's/{{ background\.pane\.bare }}ff/0a0b0cff/' "$fixture/configs/templates/fuzzel.ini"
expect_fail fuzzel-raw-colour 'fuzzel.ini' "$checker" --root "$fixture"
sed -i 's/0a0b0cff/{{ background.pane.bare }}ff/' "$fixture/configs/templates/fuzzel.ini"

sed -i 's/{{ background\.pane\.bare }}ff/{{ background.pane.bare }}fe/' "$fixture/configs/templates/fuzzel.ini"
expect_fail fuzzel-invalid-opacity 'fuzzel.ini' "$checker" --root "$fixture"
cp "$root/configs/templates/fuzzel.ini" "$fixture/configs/templates/fuzzel.ini"

sed -i 's/#ff{{ text\.normal\.bare }}/#80112233/' "$fixture/configs/templates/qt6ct-colors.conf"
expect_fail qt-raw-colour 'qt6ct-colors.conf' "$checker" --root "$fixture"
cp "$root/configs/templates/qt6ct-colors.conf" "$fixture/configs/templates/qt6ct-colors.conf"

printf '\nfuture_colors=#ff{{ text.normal.bare }}\n' >>"$fixture/configs/templates/qt6ct-colors.conf"
expect_fail qt-unclassified-colour-field 'qt6ct-colors.conf' "$checker" --root "$fixture"
cp "$root/configs/templates/qt6ct-colors.conf" "$fixture/configs/templates/qt6ct-colors.conf"

sed -i 's/theme\[main_bg\]="{{ background\.void }}"/theme[main_bg]="#112233"/' "$fixture/configs/templates/btop.theme"
expect_fail btop-raw-colour 'btop.theme' "$checker" --root "$fixture"
cp "$root/configs/templates/btop.theme" "$fixture/configs/templates/btop.theme"

sed -i 's/fg = "{{ pantheon\.charon }}"/fg = "#abcdef"/' "$fixture/configs/templates/yazi-theme.toml"
expect_fail yazi-raw-colour 'yazi-theme.toml' "$checker" --root "$fixture"
cp "$root/configs/templates/yazi-theme.toml" "$fixture/configs/templates/yazi-theme.toml"

sed -i 's/fg = "{{ pantheon\.charon }}"/fg = red/' "$fixture/configs/templates/yazi-theme.toml"
expect_fail yazi-unquoted-colour 'yazi-theme.toml' "$checker" --root "$fixture"
cp "$root/configs/templates/yazi-theme.toml" "$fixture/configs/templates/yazi-theme.toml"

sed -i '/^alpha=1\.0$/a spare_key={{ text.normal.bare }}' "$fixture/configs/templates/foot.ini"
expect_fail foot-unrecognized-key 'foot.ini' "$checker" --root "$fixture"
cp "$root/configs/templates/foot.ini" "$fixture/configs/templates/foot.ini"

sed -i '/^bright7=/d' "$fixture/configs/templates/foot.ini"
expect_fail foot-missing-required-key 'foot.ini' "$checker" --root "$fixture"
cp "$root/configs/templates/foot.ini" "$fixture/configs/templates/foot.ini"

sed -i '/^inactive_colors=/d' "$fixture/configs/templates/qt6ct-colors.conf"
expect_fail qt-missing-required-key 'qt6ct-colors.conf' "$checker" --root "$fixture"
cp "$root/configs/templates/qt6ct-colors.conf" "$fixture/configs/templates/qt6ct-colors.conf"

sed -i '/^inactive_colors=/d' "$fixture/configs/templates/qt6ct-colors.conf"
printf '\n[Other]\nvalue=unchanged\n' >>"$fixture/configs/templates/qt6ct-colors.conf"
expect_fail qt-missing-required-key-after-section 'qt6ct-colors.conf' "$checker" --root "$fixture"
cp "$root/configs/templates/qt6ct-colors.conf" "$fixture/configs/templates/qt6ct-colors.conf"

sed -i '/^    ]$/i\        Template { source: include_str!("../../../configs/templates/gtk3.css") },' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-duplicate-operand 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-redirected-source 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\/* pub fn templates() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }] } */' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-comment-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\const SPOOF: \&str = r#"" pub fn templates() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }] } ""#;' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-raw-string-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\const SPOOF: \&[u8\] = br#"" pub fn templates() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }] } ""#;' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-raw-byte-string-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\const SPOOF: \&core::ffi::CStr = cr#"" pub fn templates() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }] } ""#;' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-raw-c-string-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\mod spoof { struct Template<T> { source: T } pub fn templates() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }]; } }' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-nested-function-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\#[cfg(any())] pub fn templates() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }]; }' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-cfg-disabled-top-level-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's#../../../configs/templates/gtk3\.css#../../../configs/templates/unlisted.css#' "$fixture/crates/helm-theme/src/template.rs"
sed -i 's/^pub fn templates()/fn templates_impl()/' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\pub fn templates() -> Vec<Template> { #[cfg(any())] fn decoy() { vec![Template { source: include_str!("../../../configs/templates/gtk3.css") }, Template { source: include_str!("../../../configs/templates/gtk4.css") }, Template { source: include_str!("../../../configs/templates/foot.ini") }, Template { source: include_str!("../../../configs/templates/yazi-theme.toml") }, Template { source: include_str!("../../../configs/templates/btop.theme") }, Template { source: include_str!("../../../configs/templates/starship.toml") }, Template { source: include_str!("../../../configs/templates/fuzzel.ini") }, Template { source: include_str!("../../../configs/templates/qt6ct-colors.conf") }]; } templates_impl() }' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-disabled-inner-decoy-spoof 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

sed -i 's/::core::include_str!/include_str!/g' "$fixture/crates/helm-theme/src/template.rs"
sed -i '1i\macro_rules! include_str { ($path:literal) => { ::core::include_str!("../../../configs/templates/gtk4.css") }; }' "$fixture/crates/helm-theme/src/template.rs"
expect_fail catalogue-shadowed-include-str 'template.rs' "$checker" --root "$fixture"
cp "$root/crates/helm-theme/src/template.rs" "$fixture/crates/helm-theme/src/template.rs"

mkdir -p "$fixture/configs/templates/extra"
printf 'x = "#abcdef"\n' >"$fixture/configs/templates/extra/unclassified.conf"
expect_fail additional-template 'unclassified.conf' "$checker" --root "$fixture"
rm -f "$fixture/configs/templates/extra/unclassified.conf"
rmdir "$fixture/configs/templates/extra"

rm -f "$fixture/configs/templates/foot.ini"
expect_fail missing-template 'template inventory' "$checker" --root "$fixture"
cp "$root/configs/templates/foot.ini" "$fixture/configs/templates/foot.ini"

expect_pass ci-invokes-checker \
    grep -F 'scripts/check-colour-template-literals' "$fixture/.github/workflows/palette.yml"

printf 'PASS: %s colour-template guard fixtures\n' "$run"
