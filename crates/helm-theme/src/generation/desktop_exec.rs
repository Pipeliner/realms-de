#[derive(Debug, Clone, PartialEq, Eq)]
enum DesktopExecError {
    Empty,
    StaticArgument,
    UnsupportedFieldCode,
    MalformedEscape,
    UnsafeByte,
}

struct Token {
    bytes: Vec<u8>,
    quoted: bool,
}

fn parse_exec(value: &[u8]) -> Result<Vec<Vec<u8>>, DesktopExecError> {
    let tokens = tokenize_and_expand_once(value)?;
    if tokens.len() != 1 || tokens[0].is_empty() || tokens[0].contains(&b'=') {
        return Err(DesktopExecError::StaticArgument);
    }
    Ok(tokens)
}

fn tokenize_and_expand_once(value: &[u8]) -> Result<Vec<Vec<u8>>, DesktopExecError> {
    let value = general_unescape(value)?;
    let tokens = tokenize_whole_arguments(&value)?;
    let mut expanded = Vec::new();
    let mut payload_field_codes = 0;

    for token in tokens {
        if token.quoted && token.bytes.contains(&b'%') {
            return Err(DesktopExecError::UnsupportedFieldCode);
        }

        let mut index = 0;
        let mut bytes = Vec::new();
        let mut disappears = false;
        while index < token.bytes.len() {
            let byte = token.bytes[index];
            if byte != b'%' {
                bytes.push(byte);
                index += 1;
                continue;
            }

            let Some(&code) = token.bytes.get(index + 1) else {
                return Err(DesktopExecError::UnsupportedFieldCode);
            };
            match code {
                b'%' => bytes.push(b'%'),
                b'f' | b'F' | b'u' | b'U' if !token.quoted && token.bytes.len() == 2 => {
                    payload_field_codes += 1;
                    if payload_field_codes > 1 {
                        return Err(DesktopExecError::UnsupportedFieldCode);
                    }
                    disappears = true;
                }
                _ => return Err(DesktopExecError::UnsupportedFieldCode),
            }
            index += 2;
        }

        if !disappears {
            expanded.push(bytes);
        }
    }

    Ok(expanded)
}

fn general_unescape(value: &[u8]) -> Result<Vec<u8>, DesktopExecError> {
    let mut unescaped = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        let byte = value[index];
        if byte == b'\\' {
            let Some(&escaped) = value.get(index + 1) else {
                return Err(DesktopExecError::MalformedEscape);
            };
            match escaped {
                b's' => unescaped.push(b' '),
                b'n' => unescaped.push(b'\n'),
                b't' => unescaped.push(b'\t'),
                b'r' => unescaped.push(b'\r'),
                b'\\' => unescaped.push(b'\\'),
                _ => return Err(DesktopExecError::MalformedEscape),
            }
            index += 2;
        } else {
            unescaped.push(byte);
            index += 1;
        }
    }

    if unescaped.iter().any(|byte| !is_safe_ascii(*byte)) {
        return Err(DesktopExecError::UnsafeByte);
    }
    Ok(unescaped)
}

fn tokenize_whole_arguments(value: &[u8]) -> Result<Vec<Token>, DesktopExecError> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < value.len() {
        while value.get(index) == Some(&b' ') {
            index += 1;
        }
        if index == value.len() {
            break;
        }

        if value[index] == b'"' {
            let start = index + 1;
            index = start;
            while index < value.len() && value[index] != b'"' {
                index += 1;
            }
            if index == value.len() {
                return Err(DesktopExecError::StaticArgument);
            }
            let bytes = value[start..index].to_vec();
            index += 1;
            if index < value.len() && value[index] != b' ' {
                return Err(DesktopExecError::StaticArgument);
            }
            tokens.push(Token {
                bytes,
                quoted: true,
            });
            continue;
        }

        let start = index;
        while index < value.len() && value[index] != b' ' {
            if value[index] == b'"' || is_unquoted_reserved(value[index]) {
                return Err(DesktopExecError::StaticArgument);
            }
            index += 1;
        }
        tokens.push(Token {
            bytes: value[start..index].to_vec(),
            quoted: false,
        });
    }

    if tokens.is_empty() {
        return Err(DesktopExecError::Empty);
    }
    Ok(tokens)
}

fn is_safe_ascii(byte: u8) -> bool {
    (b' '..=b'~').contains(&byte)
}

fn is_unquoted_reserved(byte: u8) -> bool {
    matches!(
        byte,
        b'\\'
            | b'\''
            | b'>'
            | b'<'
            | b'~'
            | b'|'
            | b'&'
            | b';'
            | b'$'
            | b'*'
            | b'?'
            | b'#'
            | b'('
            | b')'
            | b'`'
    )
}

#[cfg(test)]
mod tests {
    use super::{
        admit_desktop, capture_roots, parse_exec, validate_document, AdmissionInputs,
        DesktopExecError, DesktopFileId,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn document(hidden: Option<&str>) -> String {
        let hidden = hidden
            .map(|value| format!("Hidden={value}\n"))
            .unwrap_or_default();
        format!("[Desktop Entry]\nName=Example\n{hidden}Type=Application\nExec=example\n")
    }

    fn write_entry(root: &Path, relative: &str, contents: &str) {
        let path = root.join("applications").join(relative);
        let parent = path.parent().expect("entry parent");
        fs::create_dir_all(parent).expect("create entry parent");
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .expect("secure fixture parent");
        fs::set_permissions(root.join("applications"), fs::Permissions::from_mode(0o700))
            .expect("secure applications");
        fs::write(&path, contents).expect("write desktop entry");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("secure fixture entry");
    }

    fn inputs(
        home: Option<&Path>,
        data_home: Option<&Path>,
        data_dirs: &[&Path],
    ) -> AdmissionInputs {
        for root in home
            .into_iter()
            .chain(data_home)
            .chain(data_dirs.iter().copied())
        {
            if root.exists() {
                fs::set_permissions(root, fs::Permissions::from_mode(0o700))
                    .expect("secure fixture root");
                for path in [
                    root.join(".local"),
                    root.join(".local/share"),
                    root.join("applications"),
                ] {
                    if path.exists() {
                        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                            .expect("secure fixture directory");
                    }
                }
            }
        }
        AdmissionInputs::for_test(
            home.map(Path::to_path_buf),
            data_home.map(Path::to_path_buf),
            data_dirs.iter().map(|path| path.to_path_buf()).collect(),
        )
    }

    fn id(value: &str) -> DesktopFileId {
        DesktopFileId::parse(value.as_bytes()).expect("valid desktop id")
    }

    #[test]
    fn exec_parser_accepts_only_one_payload_free_executable_argv() {
        for (value, expected) in [
            (b"profile".as_slice(), Ok(vec![b"profile".to_vec()])),
            (b"profile %%", Err(DesktopExecError::StaticArgument)),
            (b"profile %f", Ok(vec![b"profile".to_vec()])),
            (b"profile %F", Ok(vec![b"profile".to_vec()])),
            (b"profile %u", Ok(vec![b"profile".to_vec()])),
            (b"profile %U", Ok(vec![b"profile".to_vec()])),
        ] {
            assert_eq!(
                parse_exec(value),
                expected,
                "unexpected result for {value:?}"
            );
        }
    }

    #[test]
    fn exec_parser_accepts_whitespace_and_a_whole_argument_quoted_executable() {
        assert_eq!(parse_exec(b" profile "), Ok(vec![b"profile".to_vec()]));
        assert_eq!(parse_exec(b"\"profile\""), Ok(vec![b"profile".to_vec()]));
    }

    #[test]
    fn exec_parser_refuses_shell_and_field_code_ambiguity() {
        for value in [
            b"profile --flag".as_slice(),
            b"profile;id",
            b"profile$HOME",
            b"profile %c",
            b"profile %i",
            b"profile %k",
            b"profile %x",
            b"profile %Fextra",
            b"profile \\\"quoted\\\"",
            b"profile\\\\",
            b"profile\nnext",
            b"profile\0next",
        ] {
            assert!(parse_exec(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn exec_parser_refuses_static_or_unsafe_non_one_token_forms() {
        for value in [
            b"profile\\snext".as_slice(),
            b"profile %F %u",
            b"%F",
            b"profile \"%F\"",
            b"profile=override",
            b"profile\\q",
        ] {
            assert!(parse_exec(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn exec_parser_reports_empty_escape_field_and_byte_refusals() {
        assert_eq!(parse_exec(b""), Err(DesktopExecError::Empty));
        assert_eq!(parse_exec(b"   "), Err(DesktopExecError::Empty));
        assert_eq!(
            parse_exec(b"profile\\"),
            Err(DesktopExecError::MalformedEscape)
        );
        assert_eq!(
            parse_exec(b"profile %c"),
            Err(DesktopExecError::UnsupportedFieldCode)
        );
        assert_eq!(
            parse_exec(b"profile\tnext"),
            Err(DesktopExecError::UnsafeByte)
        );
    }

    mod xdg {
        use super::*;

        #[test]
        fn xdg_empty_or_unset_uses_defaults_and_requires_home_for_data_home() {
            let temp = TempDir::new().expect("temp root");
            assert!(admit_desktop(id("example.desktop"), &inputs(None, None, &[])).is_err());

            let home = temp.path().join("home");
            write_entry(
                &home.join(".local/share"),
                "example.desktop",
                &document(None),
            );
            assert_eq!(
                capture_roots(&inputs(Some(&home), None, &[])).expect("default roots"),
                vec![
                    home.join(".local/share"),
                    Path::new("/usr/local/share").to_path_buf(),
                    Path::new("/usr/share").to_path_buf(),
                ]
            );
            assert!(admit_desktop(id("example.desktop"), &inputs(Some(&home), None, &[])).is_ok());
        }

        #[test]
        fn xdg_first_matching_root_wins_and_hidden_masks_lower_root() {
            let temp = TempDir::new().expect("temp root");
            let high = temp.path().join("high");
            let low = temp.path().join("low");
            write_entry(&high, "example.desktop", &document(None));
            write_entry(&low, "example.desktop", &document(None));
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(&high), &[&low])
            )
            .is_ok());
            write_entry(&high, "example.desktop", &document(Some("true")));
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(&high), &[&low])
            )
            .is_err());
        }

        #[test]
        fn desktop_id_collision_in_one_root_refuses() {
            let temp = TempDir::new().expect("temp root");
            write_entry(temp.path(), "foo-bar.desktop", &document(None));
            write_entry(temp.path(), "foo/bar.desktop", &document(None));
            assert!(admit_desktop(
                id("foo-bar.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());
        }

        #[test]
        fn capture_refuses_symlink_unsafe_mode_duplicate_group_or_duplicate_main_key() {
            let temp = TempDir::new().expect("temp root");
            write_entry(
                temp.path(),
                "example.desktop",
                "[Desktop Entry]\nName=A\nName=B\n",
            );
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());

            write_entry(
                temp.path(),
                "example.desktop",
                "[Desktop Entry]\nName=A\n[Desktop Entry]\nType=Application\n",
            );
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());

            write_entry(temp.path(), "target.desktop", &document(None));
            let link = temp.path().join("applications/example.desktop");
            let _ = fs::remove_file(&link);
            symlink("target.desktop", &link).expect("create desktop symlink");
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());

            write_entry(temp.path(), "example.desktop", &document(None));
            fs::set_permissions(
                temp.path().join("applications/example.desktop"),
                fs::Permissions::from_mode(0o622),
            )
            .expect("make unsafe");
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());
        }

        #[test]
        fn capture_revalidates_identity_and_never_rereads_replaced_path() {
            let temp = TempDir::new().expect("temp root");
            write_entry(temp.path(), "example.desktop", &document(None));
            let source = temp.path().join("applications/example.desktop");
            let replacement = temp.path().join("applications/replacement.desktop");
            fs::write(&replacement, "[Desktop Entry]\nName=Replacement\n")
                .expect("write replacement");
            fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600))
                .expect("secure replacement");
            let inputs =
                inputs(Some(temp.path()), Some(temp.path()), &[]).with_post_read_hook(move || {
                    fs::rename(&replacement, &source).expect("replace after capture");
                });
            assert!(admit_desktop(id("example.desktop"), &inputs).is_err());
        }

        #[test]
        fn capture_enforces_root_entry_depth_file_line_and_spelling_bounds() {
            let temp = TempDir::new().expect("temp root");
            write_entry(
                temp.path(),
                "example.desktop",
                "[Desktop Entry]\nName=Example",
            );
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());

            let deep =
                std::iter::repeat_n("a", 65).collect::<Vec<_>>().join("/") + "/example.desktop";
            write_entry(temp.path(), &deep, &document(None));
            assert!(admit_desktop(id("a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-a-example.desktop"), &inputs(Some(temp.path()), Some(temp.path()), &[])).is_err());

            write_entry(
                temp.path(),
                "example.desktop",
                &format!("[Desktop Entry]\nName={}\n", "x".repeat(16 * 1024)),
            );
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());
        }

        #[test]
        fn xdg_only_initial_root_or_direct_applications_enoent_skips() {
            let temp = TempDir::new().expect("temp root");
            let absent = temp.path().join("absent");
            let low = temp.path().join("low");
            write_entry(&low, "example.desktop", &document(None));
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(&absent), &[&low])
            )
            .is_ok());
        }

        #[test]
        fn xdg_post_acquisition_enoent_refuses_without_lower_root_fallback() {
            let temp = TempDir::new().expect("temp root");
            let high = temp.path().join("high");
            let low = temp.path().join("low");
            fs::create_dir_all(high.join("applications")).expect("create high applications");
            write_entry(&low, "example.desktop", &document(None));
            let inputs =
                inputs(Some(temp.path()), Some(&high), &[&low]).with_post_acquisition_enoent();
            assert!(admit_desktop(id("example.desktop"), &inputs,).is_err());
            assert!(inputs.downstream_attempts().is_empty());
        }

        #[test]
        fn xdg_root_fd_is_boundary_and_all_descents_are_no_follow() {
            let temp = TempDir::new().expect("temp root");
            let real = temp.path().join("real");
            write_entry(&real, "example.desktop", &document(None));
            let root_link = temp.path().join("root-link");
            symlink(&real, &root_link).expect("link root");
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(&root_link), &[])
            )
            .is_err());

            let applications = real.join("applications");
            let moved = real.join("real-applications");
            fs::rename(&applications, &moved).expect("move applications");
            symlink(&moved, &applications).expect("link applications");
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(&real), &[])
            )
            .is_err());
        }

        #[test]
        fn capture_refuses_a_fifo_replacing_the_candidate_before_open() {
            let temp = TempDir::new().expect("temp root");
            write_entry(temp.path(), "example.desktop", &document(None));
            let path = temp.path().join("applications/example.desktop");
            let inputs =
                inputs(Some(temp.path()), Some(temp.path()), &[]).with_pre_open_hook(move || {
                    fs::remove_file(&path).expect("remove regular candidate");
                    rustix::fs::mkfifoat(
                        rustix::fs::CWD,
                        &path,
                        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                    )
                    .expect("replace candidate with fifo");
                });
            assert!(admit_desktop(id("example.desktop"), &inputs).is_err());
        }

        #[test]
        fn xdg_scan_refuses_on_4097th_entry_even_when_candidate_is_first() {
            let temp = TempDir::new().expect("temp root");
            write_entry(temp.path(), "000-example.desktop", &document(None));
            let applications = temp.path().join("applications");
            for number in 0..4096 {
                fs::write(applications.join(format!("z-{number:04}")), b"x").expect("write filler");
            }
            assert!(admit_desktop(
                id("000-example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());
        }

        #[test]
        fn capture_refuses_malformed_syntax_and_unterminated_final_line() {
            let temp = TempDir::new().expect("temp root");
            write_entry(temp.path(), "example.desktop", "Name=Example\n");
            assert!(admit_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());
        }

        #[test]
        fn capture_refuses_more_than_4096_lf_terminated_lines() {
            let mut document = String::from("[Desktop Entry]\n");
            for _ in 0..4096 {
                document.push_str("# comment\n");
            }
            assert!(validate_document(document.as_bytes()).is_err());
        }

        #[test]
        fn capture_refuses_non_utf8_roots_and_control_group_names() {
            let root = PathBuf::from(OsString::from_vec(b"/tmp/xdg-\xff".to_vec()));
            assert!(
                capture_roots(&AdmissionInputs::for_test(None, Some(root), Vec::new())).is_err()
            );
            assert!(validate_document(b"[Desktop\tEntry]\nName=Example\n").is_err());
        }
    }
}
use rustix::fs::{fstat, openat, statat, AtFlags, Dir, FileType, Mode, OFlags, CWD};
use rustix::io::Errno;
use rustix::process::geteuid;
use std::ffi::OsString;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

const MAX_XDG_ROOTS: usize = 64;
const MAX_XDG_SPELLING: usize = 64 * 1024;
const MAX_DESKTOP_ENTRIES: usize = 4096;
const MAX_DESKTOP_DEPTH: usize = 64;
const MAX_DESKTOP_BYTES: usize = 1024 * 1024;
const MAX_DESKTOP_LINES: usize = 4096;
const MAX_DESKTOP_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopFileId(String);

impl DesktopFileId {
    fn parse(bytes: &[u8]) -> Result<Self, AdmissionError> {
        if !(1..=255).contains(&bytes.len())
            || !bytes.ends_with(b".desktop")
            || bytes.iter().any(|byte| !matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
        {
            return Err(AdmissionError::Refused);
        }
        Ok(Self(
            String::from_utf8(bytes.to_vec()).map_err(|_| AdmissionError::Refused)?,
        ))
    }
}

struct AdmissionInputs {
    home: Option<PathBuf>,
    data_home: Option<PathBuf>,
    data_dirs: Vec<PathBuf>,
    #[cfg(test)]
    post_acquisition_enoent: bool,
    #[cfg(test)]
    audit: std::sync::Mutex<Vec<AdmissionAuditEvent>>,
    #[cfg(test)]
    pre_open_hook: Option<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    post_read_hook: Option<Box<dyn Fn() + Send + Sync>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionAuditEvent {
    PlanMadeAvailable,
}

impl AdmissionInputs {
    #[cfg(test)]
    fn for_test(
        home: Option<PathBuf>,
        data_home: Option<PathBuf>,
        data_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            home,
            data_home,
            data_dirs,
            post_acquisition_enoent: false,
            audit: std::sync::Mutex::new(Vec::new()),
            pre_open_hook: None,
            post_read_hook: None,
        }
    }

    #[cfg(test)]
    fn with_post_acquisition_enoent(mut self) -> Self {
        self.post_acquisition_enoent = true;
        self
    }

    #[cfg(test)]
    fn with_pre_open_hook(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.pre_open_hook = Some(Box::new(hook));
        self
    }

    #[cfg(test)]
    fn with_post_read_hook(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.post_read_hook = Some(Box::new(hook));
        self
    }

    #[cfg(test)]
    fn downstream_attempts(&self) -> Vec<AdmissionAuditEvent> {
        self.audit.lock().expect("test audit lock").clone()
    }

    #[cfg(test)]
    fn record_plan_available(&self) {
        self.audit
            .lock()
            .expect("test audit lock")
            .push(AdmissionAuditEvent::PlanMadeAvailable);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    size: i64,
    mtime: (i64, u64),
    ctime: (i64, u64),
}

impl FileIdentity {
    fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            dev: stat.st_dev,
            ino: stat.st_ino,
            size: stat.st_size,
            mtime: (stat.st_mtime, stat.st_mtime_nsec),
            ctime: (stat.st_ctime, stat.st_ctime_nsec),
        }
    }
}

#[derive(Debug)]
struct AdmittedDesktopPlan {
    bytes: Vec<u8>,
    #[allow(dead_code)]
    identity: FileIdentity,
}

impl AdmittedDesktopPlan {
    #[cfg(test)]
    fn desktop_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionError {
    Refused,
}

impl From<Errno> for AdmissionError {
    fn from(_: Errno) -> Self {
        Self::Refused
    }
}

fn admit_desktop(
    id: DesktopFileId,
    inputs: &AdmissionInputs,
) -> Result<AdmittedDesktopPlan, AdmissionError> {
    let roots = capture_roots(inputs)?;
    for root_path in roots {
        let root = match openat(CWD, &root_path, directory_flags(), Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => continue,
            Err(_) => return Err(AdmissionError::Refused),
        };
        validate_directory(&root)?;
        let applications = match openat(&root, "applications", directory_flags(), Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => continue,
            Err(_) => return Err(AdmissionError::Refused),
        };
        validate_directory(&applications)?;
        #[cfg(test)]
        if inputs.post_acquisition_enoent {
            return Err(AdmissionError::Refused);
        }
        let mut state = ScanState {
            entries: 0,
            matches: Vec::new(),
        };
        scan_directory(&applications, &id, &mut state, Vec::new())?;
        match state.matches.len() {
            0 => continue,
            1 => {
                let plan = capture_candidate(&applications, &state.matches[0], inputs)?;
                match hidden_value(&plan.bytes)? {
                    Some(true) => return Err(AdmissionError::Refused),
                    _ => {
                        #[cfg(test)]
                        inputs.record_plan_available();
                        return Ok(plan);
                    }
                }
            }
            _ => return Err(AdmissionError::Refused),
        }
    }
    Err(AdmissionError::Refused)
}

fn capture_roots(inputs: &AdmissionInputs) -> Result<Vec<PathBuf>, AdmissionError> {
    let mut roots = Vec::new();
    if let Some(home) = &inputs.data_home {
        roots.push(home.clone());
    } else {
        let home = inputs.home.as_ref().ok_or(AdmissionError::Refused)?;
        roots.push(home.join(".local/share"));
    }
    if inputs.data_dirs.is_empty() {
        roots.extend([
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ]);
    } else {
        roots.extend(inputs.data_dirs.iter().cloned());
    }
    if roots.len() > MAX_XDG_ROOTS
        || roots
            .iter()
            .map(|path| path.as_os_str().as_bytes().len())
            .sum::<usize>()
            > MAX_XDG_SPELLING
    {
        return Err(AdmissionError::Refused);
    }
    for root in &roots {
        if !root.is_absolute() || root.as_os_str().as_bytes().is_empty() || root.to_str().is_none()
        {
            return Err(AdmissionError::Refused);
        }
    }
    Ok(roots)
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn validate_directory(fd: &OwnedFd) -> Result<(), AdmissionError> {
    let stat = fstat(fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory || !safe_owner_mode(&stat) {
        return Err(AdmissionError::Refused);
    }
    Ok(())
}

fn safe_owner_mode(stat: &rustix::fs::Stat) -> bool {
    (stat.st_uid == 0 || stat.st_uid == geteuid().as_raw()) && (stat.st_mode & 0o022) == 0
}

struct ScanState {
    entries: usize,
    matches: Vec<Vec<OsString>>,
}

fn scan_directory(
    fd: &OwnedFd,
    id: &DesktopFileId,
    state: &mut ScanState,
    prefix: Vec<OsString>,
) -> Result<(), AdmissionError> {
    let mut directory = Dir::read_from(fd)?;
    let mut names = Vec::new();
    while let Some(entry) = directory.read() {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        state.entries += 1;
        if state.entries > MAX_DESKTOP_ENTRIES {
            return Err(AdmissionError::Refused);
        }
        names.push(OsString::from_vec(name.to_vec()));
    }
    names.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });
    for name in names {
        let stat = statat(fd, &name, AtFlags::SYMLINK_NOFOLLOW)?;
        let kind = FileType::from_raw_mode(stat.st_mode);
        let mut relative = prefix.clone();
        relative.push(name.clone());
        if relative.len() > MAX_DESKTOP_DEPTH {
            return Err(AdmissionError::Refused);
        }
        if kind == FileType::Directory {
            if !safe_owner_mode(&stat) {
                return Err(AdmissionError::Refused);
            }
            let child = openat(fd, &name, directory_flags(), Mode::empty())?;
            validate_directory(&child)?;
            scan_directory(&child, id, state, relative)?;
        } else if kind == FileType::RegularFile {
            if flattened_id(&relative) == id.0.as_bytes() {
                state.matches.push(relative);
            }
        } else {
            return Err(AdmissionError::Refused);
        }
    }
    Ok(())
}

fn flattened_id(components: &[OsString]) -> Vec<u8> {
    let mut id = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            id.push(b'-');
        }
        id.extend_from_slice(component.as_os_str().as_bytes());
    }
    id
}

fn capture_candidate(
    applications: &OwnedFd,
    relative: &[OsString],
    _inputs: &AdmissionInputs,
) -> Result<AdmittedDesktopPlan, AdmissionError> {
    let (name, parents) = relative.split_last().ok_or(AdmissionError::Refused)?;
    let mut parent = openat(applications, ".", directory_flags(), Mode::empty())?;
    for component in parents {
        parent = openat(&parent, component, directory_flags(), Mode::empty())?;
        validate_directory(&parent)?;
    }
    let before = statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile
        || !safe_owner_mode(&before)
        || before.st_size < 0
        || before.st_size as usize > MAX_DESKTOP_BYTES
    {
        return Err(AdmissionError::Refused);
    }
    #[cfg(test)]
    if let Some(hook) = &_inputs.pre_open_hook {
        hook();
    }
    let fd = openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let mut file = std::fs::File::from(fd);
    let identity = FileIdentity::from_stat(&before);
    let opened = fstat(&file)?;
    if FileType::from_raw_mode(opened.st_mode) != FileType::RegularFile
        || !safe_owner_mode(&opened)
        || FileIdentity::from_stat(&opened) != identity
    {
        return Err(AdmissionError::Refused);
    }
    let mut bytes = vec![0; identity.size as usize];
    (&mut file)
        .take(identity.size as u64)
        .read_exact(&mut bytes)
        .map_err(|_| AdmissionError::Refused)?;
    #[cfg(test)]
    if let Some(hook) = &_inputs.post_read_hook {
        hook();
    }
    if FileIdentity::from_stat(&fstat(&file)?) != identity
        || FileIdentity::from_stat(&statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)?) != identity
    {
        return Err(AdmissionError::Refused);
    }
    validate_document(&bytes)?;
    Ok(AdmittedDesktopPlan { bytes, identity })
}

fn validate_document(bytes: &[u8]) -> Result<(), AdmissionError> {
    if bytes.is_empty()
        || !bytes.ends_with(b"\n")
        || bytes.starts_with(&[0xef, 0xbb, 0xbf])
        || bytes.iter().any(|byte| matches!(byte, 0 | b'\r'))
    {
        return Err(AdmissionError::Refused);
    }
    let source = std::str::from_utf8(bytes).map_err(|_| AdmissionError::Refused)?;
    let mut groups =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    let mut current = None::<String>;
    let mut line_count = 0usize;
    for line in source.split_terminator('\n') {
        line_count += 1;
        if line_count > MAX_DESKTOP_LINES {
            return Err(AdmissionError::Refused);
        }
        if line.len() > MAX_DESKTOP_LINE_BYTES {
            return Err(AdmissionError::Refused);
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            if name.is_empty()
                || name.trim() != name
                || !name.bytes().all(|byte| (b' '..=b'~').contains(&byte))
                || name.bytes().any(|byte| matches!(byte, b'[' | b']' | b'='))
                || groups.contains_key(name)
            {
                return Err(AdmissionError::Refused);
            }
            groups.insert(name.to_string(), std::collections::BTreeSet::new());
            current = Some(name.to_string());
            continue;
        }
        let (key, _) = line.split_once('=').ok_or(AdmissionError::Refused)?;
        let group = current.as_ref().ok_or(AdmissionError::Refused)?;
        if !valid_key(key)
            || !groups
                .get_mut(group)
                .expect("current group")
                .insert(key.to_string())
        {
            return Err(AdmissionError::Refused);
        }
    }
    if !groups.contains_key("Desktop Entry") {
        return Err(AdmissionError::Refused);
    }
    Ok(())
}

fn valid_key(key: &str) -> bool {
    let (base, locale) = match key.split_once('[') {
        Some((base, locale))
            if locale.ends_with(']') && !locale[..locale.len() - 1].contains('[') =>
        {
            (base, Some(&locale[..locale.len() - 1]))
        }
        Some(_) => return false,
        None => (key, None),
    };
    !base.is_empty()
        && base
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && locale.is_none_or(|locale| {
            !locale.is_empty()
                && locale.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_' | b'.' | b'-')
                })
        })
}

fn hidden_value(bytes: &[u8]) -> Result<Option<bool>, AdmissionError> {
    let source = std::str::from_utf8(bytes).map_err(|_| AdmissionError::Refused)?;
    let mut main = false;
    for line in source.split_terminator('\n') {
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
            continue;
        }
        if main && line.starts_with("Hidden=") {
            return match &line[7..] {
                "true" => Ok(Some(true)),
                "false" => Ok(Some(false)),
                _ => Err(AdmissionError::Refused),
            };
        }
    }
    Ok(None)
}
