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
        absolute_components, admit_desktop, capture_base_environment, capture_desktop,
        capture_roots, capture_working_directory, parse_exec, parse_try_exec, resolve_executable,
        validate_document, validate_main_group, validate_path, AdmissionAuditEvent,
        AdmissionInputs, DesktopExecError, DesktopFileId,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
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

    #[test]
    fn dbus_refusal_after_structural_validation() {
        let temp = TempDir::new().expect("temp root");
        write_entry(
            temp.path(),
            "example.desktop",
            "[Desktop Entry]\nType=Application\nName=Example\nDBusActivatable=true\nExec=invalid;exec\n",
        );
        let admission_inputs = inputs(Some(temp.path()), Some(temp.path()), &[]);
        assert!(admit_desktop(id("example.desktop"), &admission_inputs).is_err());
        assert_eq!(
            admission_inputs.downstream_attempts(),
            vec![
                AdmissionAuditEvent::MainGroupValidated,
                AdmissionAuditEvent::DbusDecision,
            ]
        );

        for document in [
            "[Desktop Entry]\nType=Link\nName=Example\nDBusActivatable=true\n",
            "[Desktop Entry]\nType=Application\nName=\nDBusActivatable=true\n",
            "[Desktop Entry]\nType=Application\nName=Example\nHidden=true\nDBusActivatable=true\n",
            "[Desktop Entry]\nType=Application\nName=Example\nNoDisplay=true\nDBusActivatable=true\n",
            "[Desktop Entry]\nType=Application\nName=Example\nTerminal=true\nDBusActivatable=true\n",
        ] {
            write_entry(temp.path(), "example.desktop", document);
            let admission_inputs = inputs(Some(temp.path()), Some(temp.path()), &[]);
            assert!(admit_desktop(id("example.desktop"), &admission_inputs).is_err());
            assert!(admission_inputs.downstream_attempts().is_empty());
        }
    }

    #[test]
    fn static_preflight_refuses_without_generation_effect() {
        let temp = TempDir::new().expect("temp root");
        for (document, expected) in [
            (
                "[Desktop Entry]\nType=Application\nName=Example\n",
                vec![
                    AdmissionAuditEvent::MainGroupValidated,
                    AdmissionAuditEvent::DbusDecision,
                    AdmissionAuditEvent::ExecAccessed,
                ],
            ),
            (
                "[Desktop Entry]\nType=Application\nName=Example\nExec=invalid;exec\n",
                vec![
                    AdmissionAuditEvent::MainGroupValidated,
                    AdmissionAuditEvent::DbusDecision,
                    AdmissionAuditEvent::ExecAccessed,
                ],
            ),
            (
                "[Desktop Entry]\nType=Application\nName=Example\nExec=example\nTryExec=two words\n",
                vec![
                    AdmissionAuditEvent::MainGroupValidated,
                    AdmissionAuditEvent::DbusDecision,
                    AdmissionAuditEvent::ExecAccessed,
                    AdmissionAuditEvent::TryExecAccessed,
                ],
            ),
        ] {
            write_entry(temp.path(), "example.desktop", document);
            let admission_inputs = inputs(Some(temp.path()), Some(temp.path()), &[]);
            assert!(admit_desktop(id("example.desktop"), &admission_inputs).is_err());
            assert_eq!(admission_inputs.downstream_attempts(), expected);
        }
    }

    #[test]
    fn static_preflight_refuses_invalid_base_environment_before_plan_handoff() {
        let temp = TempDir::new().expect("temp root");
        write_entry(temp.path(), "example.desktop", &document(None));
        for environment in [
            vec![
                OsString::from("PATH=/usr/bin"),
                OsString::from("LD_PRELOAD=x"),
            ],
            vec![OsString::from("HOME=/home/example")],
            vec![OsString::from("PATH=relative")],
            vec![OsString::from("PATH=/usr/bin:/usr/bin")],
        ] {
            let admission_inputs =
                inputs(Some(temp.path()), Some(temp.path()), &[]).with_environment(environment);
            assert!(admit_desktop(id("example.desktop"), &admission_inputs).is_err());
            assert_eq!(
                admission_inputs.downstream_attempts(),
                vec![
                    AdmissionAuditEvent::MainGroupValidated,
                    AdmissionAuditEvent::DbusDecision,
                    AdmissionAuditEvent::ExecAccessed,
                    AdmissionAuditEvent::TryExecAccessed,
                    AdmissionAuditEvent::BaseEnvironmentAccessed,
                ]
            );
        }
    }

    #[test]
    fn static_preflight_accepts_root_as_a_normalized_path_component() {
        let temp = TempDir::new().expect("temp root");
        write_entry(temp.path(), "example.desktop", &document(None));
        assert!(validate_path("/").is_ok());
        assert_eq!(
            absolute_components("/").expect("root components"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn static_preflight_refuses_an_exec_name_missing_from_captured_path() {
        let temp = TempDir::new().expect("temp root");
        write_entry(
            temp.path(),
            "example.desktop",
            "[Desktop Entry]\nType=Application\nName=Example\nExec=does-not-exist\n",
        );
        let admission_inputs = inputs(Some(temp.path()), Some(temp.path()), &[])
            .with_environment(vec![OsString::from("PATH=/usr/bin")]);

        assert!(admit_desktop(id("example.desktop"), &admission_inputs).is_err());
    }

    #[test]
    fn elf_preflight_refuses_a_header_with_an_invalid_elf_version_field() {
        let mut header = [0_u8; 64];
        header[..4].copy_from_slice(b"\x7fELF");
        header[4] = 2;
        header[5] = 1;
        header[6] = 1;
        header[16..18].copy_from_slice(&3_u16.to_le_bytes());
        header[18..20].copy_from_slice(
            &(if cfg!(target_arch = "x86_64") {
                62_u16
            } else {
                183_u16
            })
            .to_le_bytes(),
        );
        header[20..24].copy_from_slice(&0_u32.to_le_bytes());

        assert!(super::validate_elf_header(&header).is_err());
    }

    #[test]
    fn try_exec_preflight_refuses_non_name_punctuation() {
        let document = validate_document(
            b"[Desktop Entry]\nType=Application\nName=Example\nExec=example\nTryExec=example;unsafe\n",
        )
        .expect("well-formed desktop document");
        let inputs = inputs(None, None, &[]);

        assert!(parse_try_exec(&document, &inputs).is_err());
    }

    #[test]
    fn executable_preflight_requires_proven_effective_access() {
        assert!(super::effective_access_is_proven(
            Err(super::Errno::ACCESS),
            Ok(())
        ));
        assert!(!super::effective_access_is_proven(Ok(()), Ok(())));
        assert!(!super::effective_access_is_proven(
            Err(super::Errno::NOSYS),
            Ok(())
        ));
        assert!(!super::effective_access_is_proven(
            Err(super::Errno::ACCESS),
            Err(super::Errno::ACCESS)
        ));
    }

    #[test]
    #[ignore = "requires a non-root NixOS VM with a root-owned ELF fixture"]
    fn nixos_vm_static_preflight_admits_root_owned_elf_from_non_root_user() {
        assert_ne!(
            rustix::process::geteuid().as_raw(),
            0,
            "VM test user is non-root"
        );
        let executable = std::env::var_os("HELM_DESKTOP_EXEC_TEST_ELF")
            .expect("NixOS VM supplies the root-owned ELF fixture");
        let executable = executable
            .into_string()
            .expect("VM fixture executable is UTF-8");
        let executable_path = Path::new(&executable);
        let executable_directory = executable_path.parent().expect("fixture parent");
        let executable_name = executable_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fixture executable name");
        assert_eq!(
            fs::metadata(executable_path)
                .expect("fixture metadata")
                .uid(),
            0,
            "the controlled fixture must be root-owned"
        );

        let temp = TempDir::new().expect("temp root");
        write_entry(
            temp.path(),
            "example.desktop",
            &format!(
                "[Desktop Entry]\nType=Application\nName=Example\nExec={executable_name}\nTryExec={executable_name}\nPath=/\n"
            ),
        );
        let inputs = inputs(Some(temp.path()), Some(temp.path()), &[]).with_environment(vec![
            OsString::from(format!("PATH={}", executable_directory.display())),
            OsString::from("LANG=C.UTF-8"),
        ]);

        let captured =
            capture_desktop(id("example.desktop"), &inputs).expect("VM fixture desktop capture");
        validate_main_group(&captured.document, &inputs).expect("VM fixture main group");
        let argv = parse_exec(
            captured
                .document
                .groups
                .get("Desktop Entry")
                .and_then(|main| main.get("Exec"))
                .expect("VM fixture Exec")
                .as_bytes(),
        )
        .expect("VM fixture Exec syntax");
        let base_environment =
            capture_base_environment(&inputs).expect("VM fixture base environment");
        let _exec = resolve_executable(&argv[0], &base_environment.path_directories)
            .expect("VM fixture Exec static preflight");
        let _try_exec = parse_try_exec(&captured.document, &inputs)
            .expect("VM fixture TryExec syntax")
            .as_deref()
            .map(|value| resolve_executable(value, &base_environment.path_directories))
            .transpose()
            .expect("VM fixture TryExec static preflight");
        let _cwd =
            capture_working_directory(&captured.document).expect("VM fixture working directory");

        let plan = admit_desktop(id("example.desktop"), &inputs).expect("accepted VM plan");
        assert_eq!(plan.argv, vec![executable_name.as_bytes().to_vec()]);
        assert_eq!(
            plan.executable.identity,
            super::FileIdentity::from_stat(
                &rustix::fs::fstat(&plan.executable.fd).expect("exec stat")
            )
        );
        assert_eq!(
            plan.try_exec.as_ref().expect("TryExec descriptor").identity,
            super::FileIdentity::from_stat(
                &rustix::fs::fstat(&plan.try_exec.as_ref().expect("TryExec descriptor").fd)
                    .expect("TryExec stat")
            )
        );
        assert_eq!(
            plan.base_environment
                .entries
                .iter()
                .map(|entry| entry.as_os_str().as_bytes().to_vec())
                .collect::<Vec<_>>(),
            vec![
                b"LANG=C.UTF-8".to_vec(),
                format!("PATH={}", executable_directory.display()).into_bytes(),
            ]
        );
        assert_eq!(
            plan.cwd.identity,
            super::FileIdentity::from_stat(&rustix::fs::fstat(&plan.cwd.fd).expect("cwd stat"))
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
            assert!(
                capture_desktop(id("example.desktop"), &inputs(Some(&home), None, &[])).is_ok()
            );
        }

        #[test]
        fn xdg_first_matching_root_wins_and_hidden_masks_lower_root() {
            let temp = TempDir::new().expect("temp root");
            let high = temp.path().join("high");
            let low = temp.path().join("low");
            write_entry(&high, "example.desktop", &document(None));
            write_entry(&low, "example.desktop", &document(None));
            assert!(capture_desktop(
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
            assert!(capture_desktop(
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
            assert!(capture_desktop(
                id("example.desktop"),
                &inputs(Some(temp.path()), Some(temp.path()), &[])
            )
            .is_err());

            write_entry(
                temp.path(),
                "example.desktop",
                "[Desktop Entry]\nName=A\n[Desktop Entry]\nType=Application\n",
            );
            assert!(capture_desktop(
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
            assert!(capture_desktop(
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
use rustix::fs::{
    accessat, fstat, openat, statat, Access, AtFlags, Dir, FileType, Mode, OFlags, CWD,
};
use rustix::io::pread;
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
    environment: Vec<OsString>,
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
    MainGroupValidated,
    DbusDecision,
    ExecAccessed,
    TryExecAccessed,
    BaseEnvironmentAccessed,
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
            environment: vec![OsString::from("PATH=/usr/bin")],
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
    fn with_environment(mut self, environment: Vec<OsString>) -> Self {
        self.environment = environment;
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

    #[cfg(test)]
    fn record_main_group_validated(&self) {
        self.audit
            .lock()
            .expect("test audit lock")
            .push(AdmissionAuditEvent::MainGroupValidated);
    }

    #[cfg(test)]
    fn record_dbus_decision(&self) {
        self.audit
            .lock()
            .expect("test audit lock")
            .push(AdmissionAuditEvent::DbusDecision);
    }

    #[cfg(test)]
    fn record_exec_accessed(&self) {
        self.audit
            .lock()
            .expect("test audit lock")
            .push(AdmissionAuditEvent::ExecAccessed);
    }

    #[cfg(test)]
    fn record_try_exec_accessed(&self) {
        self.audit
            .lock()
            .expect("test audit lock")
            .push(AdmissionAuditEvent::TryExecAccessed);
    }

    #[cfg(test)]
    fn record_base_environment_accessed(&self) {
        self.audit
            .lock()
            .expect("test audit lock")
            .push(AdmissionAuditEvent::BaseEnvironmentAccessed);
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
struct CapturedDesktop {
    bytes: Vec<u8>,
    document: DesktopDocument,
    identity: FileIdentity,
}

impl CapturedDesktop {
    #[cfg(test)]
    fn desktop_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug)]
struct HeldDirectory {
    fd: OwnedFd,
    identity: FileIdentity,
}

#[derive(Debug)]
struct HeldExecutable {
    fd: OwnedFd,
    identity: FileIdentity,
    header: [u8; 64],
}

#[derive(Debug)]
struct BaseEnvironment {
    entries: Vec<OsString>,
    path_directories: Vec<HeldDirectory>,
}

#[derive(Debug)]
struct StaticPreflight {
    argv: Vec<Vec<u8>>,
    executable: HeldExecutable,
    try_exec: Option<HeldExecutable>,
    cwd: HeldDirectory,
    base_environment: BaseEnvironment,
}

#[derive(Debug)]
struct AdmittedDesktopPlan {
    bytes: Vec<u8>,
    document: DesktopDocument,
    identity: FileIdentity,
    argv: Vec<Vec<u8>>,
    executable: HeldExecutable,
    try_exec: Option<HeldExecutable>,
    cwd: HeldDirectory,
    base_environment: BaseEnvironment,
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
    let captured = capture_desktop(id, inputs)?;
    validate_main_group(&captured.document, inputs)?;
    let static_plan = preflight_static(&captured.document, inputs)?;
    #[cfg(test)]
    inputs.record_plan_available();
    Ok(AdmittedDesktopPlan {
        bytes: captured.bytes,
        document: captured.document,
        identity: captured.identity,
        argv: static_plan.argv,
        executable: static_plan.executable,
        try_exec: static_plan.try_exec,
        cwd: static_plan.cwd,
        base_environment: static_plan.base_environment,
    })
}

fn capture_desktop(
    id: DesktopFileId,
    inputs: &AdmissionInputs,
) -> Result<CapturedDesktop, AdmissionError> {
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
                return Ok(plan);
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
) -> Result<CapturedDesktop, AdmissionError> {
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
    let document = validate_document(&bytes)?;
    Ok(CapturedDesktop {
        bytes,
        document,
        identity,
    })
}

#[derive(Debug)]
struct DesktopDocument {
    groups: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

fn validate_document(bytes: &[u8]) -> Result<DesktopDocument, AdmissionError> {
    if bytes.is_empty()
        || !bytes.ends_with(b"\n")
        || bytes.starts_with(&[0xef, 0xbb, 0xbf])
        || bytes.iter().any(|byte| matches!(byte, 0 | b'\r'))
    {
        return Err(AdmissionError::Refused);
    }
    let source = std::str::from_utf8(bytes).map_err(|_| AdmissionError::Refused)?;
    let mut groups =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, String>>::new();
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
            groups.insert(name.to_string(), std::collections::BTreeMap::new());
            current = Some(name.to_string());
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(AdmissionError::Refused)?;
        let group = current.as_ref().ok_or(AdmissionError::Refused)?;
        if !valid_key(key)
            || !groups
                .get_mut(group)
                .expect("current group")
                .insert(key.to_string(), value.to_string())
                .is_none()
        {
            return Err(AdmissionError::Refused);
        }
    }
    if !groups.contains_key("Desktop Entry") {
        return Err(AdmissionError::Refused);
    }
    Ok(DesktopDocument { groups })
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

fn validate_main_group(
    document: &DesktopDocument,
    _inputs: &AdmissionInputs,
) -> Result<(), AdmissionError> {
    let main = document
        .groups
        .get("Desktop Entry")
        .ok_or(AdmissionError::Refused)?;
    if main.get("Type").map(String::as_str) != Some("Application")
        || main.get("Name").is_none_or(String::is_empty)
    {
        return Err(AdmissionError::Refused);
    }
    for key in ["Hidden", "NoDisplay", "Terminal"] {
        if let Some(value) = main.get(key) {
            if value != "false" {
                return Err(AdmissionError::Refused);
            }
        }
    }
    #[cfg(test)]
    _inputs.record_main_group_validated();
    #[cfg(test)]
    _inputs.record_dbus_decision();
    match main.get("DBusActivatable").map(String::as_str) {
        None | Some("false") => Ok(()),
        Some("true") | Some(_) => Err(AdmissionError::Refused),
    }
}

fn preflight_static(
    document: &DesktopDocument,
    _inputs: &AdmissionInputs,
) -> Result<StaticPreflight, AdmissionError> {
    #[cfg(test)]
    _inputs.record_exec_accessed();
    let exec = document
        .groups
        .get("Desktop Entry")
        .and_then(|main| main.get("Exec"))
        .ok_or(AdmissionError::Refused)?;
    let argv = parse_exec(exec.as_bytes()).map_err(|_| AdmissionError::Refused)?;
    let try_exec = parse_try_exec(document, _inputs)?;
    let base_environment = capture_base_environment(_inputs)?;
    let executable = resolve_executable(&argv[0], &base_environment.path_directories)?;
    let try_exec = try_exec
        .as_deref()
        .map(|value| resolve_executable(value, &base_environment.path_directories))
        .transpose()?;
    let cwd = capture_working_directory(document)?;
    Ok(StaticPreflight {
        argv,
        executable,
        try_exec,
        cwd,
        base_environment,
    })
}

fn parse_try_exec(
    document: &DesktopDocument,
    _inputs: &AdmissionInputs,
) -> Result<Option<Vec<u8>>, AdmissionError> {
    #[cfg(test)]
    _inputs.record_try_exec_accessed();
    let Some(try_exec) = document
        .groups
        .get("Desktop Entry")
        .and_then(|main| main.get("TryExec"))
    else {
        return Ok(None);
    };
    if try_exec.is_empty()
        || !try_exec.is_ascii()
        || try_exec
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'"' | b'%' | b'\\'))
    {
        return Err(AdmissionError::Refused);
    }
    if try_exec.contains('/') && !is_normalized_absolute(try_exec) {
        return Err(AdmissionError::Refused);
    }
    if !try_exec.contains('/')
        && !try_exec
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        return Err(AdmissionError::Refused);
    }
    Ok(Some(try_exec.as_bytes().to_vec()))
}

fn capture_base_environment(_inputs: &AdmissionInputs) -> Result<BaseEnvironment, AdmissionError> {
    #[cfg(test)]
    _inputs.record_base_environment_accessed();
    const ALLOWED_NAMES: &[&str] = &[
        "HOME",
        "PATH",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "LANG",
        "LC_CTYPE",
        "LC_MESSAGES",
        "XDG_CONFIG_DIRS",
        "XDG_DATA_DIRS",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ];
    if _inputs.environment.len() > 256
        || _inputs
            .environment
            .iter()
            .map(|entry| entry.as_os_str().as_bytes().len())
            .sum::<usize>()
            > 128 * 1024
    {
        return Err(AdmissionError::Refused);
    }
    let mut names = std::collections::BTreeSet::new();
    let mut path = None;
    let mut entries = Vec::<OsString>::with_capacity(_inputs.environment.len());
    for entry in &_inputs.environment {
        let entry = entry.to_str().ok_or(AdmissionError::Refused)?;
        let (name, value) = entry.split_once('=').ok_or(AdmissionError::Refused)?;
        if !valid_environment_name(name)
            || name.starts_with("LD_")
            || !ALLOWED_NAMES.contains(&name)
            || !names.insert(name)
        {
            return Err(AdmissionError::Refused);
        }
        if name == "PATH" {
            path = Some(value);
        }
        entries.push(entry.to_owned().into());
    }
    let path = path.ok_or(AdmissionError::Refused)?;
    validate_path(path)?;
    let mut path_directories = Vec::new();
    for component in path.split(':') {
        path_directories.push(open_static_directory_absolute(component)?);
    }
    entries.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });
    Ok(BaseEnvironment {
        entries,
        path_directories,
    })
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn validate_path(path: &str) -> Result<(), AdmissionError> {
    if path.is_empty() || path.len() > 32 * 1024 {
        return Err(AdmissionError::Refused);
    }
    let entries = path.split(':').collect::<Vec<_>>();
    if entries.len() > 64 {
        return Err(AdmissionError::Refused);
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        if !is_normalized_absolute(entry) || !seen.insert(entry) {
            return Err(AdmissionError::Refused);
        }
    }
    Ok(())
}

fn capture_working_directory(document: &DesktopDocument) -> Result<HeldDirectory, AdmissionError> {
    let path = document
        .groups
        .get("Desktop Entry")
        .and_then(|main| main.get("Path"))
        .filter(|path| !path.is_empty())
        .map(String::as_str)
        .unwrap_or("/");
    open_static_directory_absolute(path)
}

fn open_static_directory_absolute(path: &str) -> Result<HeldDirectory, AdmissionError> {
    let components = absolute_components(path)?;
    let mut fd = openat(CWD, "/", directory_flags(), Mode::empty())?;
    validate_static_directory(&fd)?;
    for component in components {
        fd = openat(&fd, component, directory_flags(), Mode::empty())?;
        validate_static_directory(&fd)?;
    }
    let identity = FileIdentity::from_stat(&fstat(&fd)?);
    Ok(HeldDirectory { fd, identity })
}

fn absolute_components(path: &str) -> Result<Vec<&str>, AdmissionError> {
    if !is_normalized_absolute(path) {
        return Err(AdmissionError::Refused);
    }
    if path == "/" {
        return Ok(Vec::new());
    }
    Ok(path[1..].split('/').collect())
}

fn validate_static_directory(fd: &OwnedFd) -> Result<(), AdmissionError> {
    let stat = fstat(fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != 0
        || (stat.st_mode & 0o022) != 0
    {
        return Err(AdmissionError::Refused);
    }
    Ok(())
}

fn resolve_executable(
    value: &[u8],
    path_directories: &[HeldDirectory],
) -> Result<HeldExecutable, AdmissionError> {
    let value = std::str::from_utf8(value).map_err(|_| AdmissionError::Refused)?;
    if value.contains('/') {
        let (parent, name) = value.rsplit_once('/').ok_or(AdmissionError::Refused)?;
        if name.is_empty() || !is_normalized_absolute(value) {
            return Err(AdmissionError::Refused);
        }
        return open_static_executable(
            &open_static_directory_absolute(if parent.is_empty() { "/" } else { parent })?,
            name,
        );
    }
    if value.is_empty() || value.bytes().any(|byte| !(b' '..=b'~').contains(&byte)) {
        return Err(AdmissionError::Refused);
    }
    for directory in path_directories {
        match statat(&directory.fd, value, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return open_static_executable(directory, value),
            Err(Errno::NOENT) => continue,
            Err(_) => return Err(AdmissionError::Refused),
        }
    }
    Err(AdmissionError::Refused)
}

fn open_static_executable(
    parent: &HeldDirectory,
    name: &str,
) -> Result<HeldExecutable, AdmissionError> {
    let before = statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW)?;
    validate_executable_stat(&before)?;
    let flags = AtFlags::SYMLINK_NOFOLLOW | AtFlags::EACCESS;
    let write_access = accessat(&parent.fd, name, Access::WRITE_OK, flags);
    let execute_access = accessat(&parent.fd, name, Access::EXEC_OK, flags);
    #[cfg(test)]
    eprintln!("desktop Exec access pre-open: write={write_access:?} execute={execute_access:?}");
    if !effective_access_is_proven(write_access, execute_access) {
        return Err(AdmissionError::Refused);
    }
    let fd = openat(
        &parent.fd,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let identity = FileIdentity::from_stat(&before);
    let opened = fstat(&fd)?;
    if FileIdentity::from_stat(&opened) != identity || validate_executable_stat(&opened).is_err() {
        return Err(AdmissionError::Refused);
    }
    let write_access = accessat(&parent.fd, name, Access::WRITE_OK, flags);
    let execute_access = accessat(&parent.fd, name, Access::EXEC_OK, flags);
    #[cfg(test)]
    eprintln!("desktop Exec access post-open: write={write_access:?} execute={execute_access:?}");
    if !effective_access_is_proven(write_access, execute_access) {
        return Err(AdmissionError::Refused);
    }
    let mut header = [0_u8; 64];
    if pread(&fd, &mut header, 0)? != header.len() {
        return Err(AdmissionError::Refused);
    }
    if FileIdentity::from_stat(&fstat(&fd)?) != identity {
        return Err(AdmissionError::Refused);
    }
    validate_elf_header(&header)?;
    Ok(HeldExecutable {
        fd,
        identity,
        header,
    })
}

fn effective_access_is_proven(
    write_access: Result<(), Errno>,
    execute_access: Result<(), Errno>,
) -> bool {
    matches!(write_access, Err(Errno::ACCESS)) && execute_access.is_ok()
}

fn validate_executable_stat(stat: &rustix::fs::Stat) -> Result<(), AdmissionError> {
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != 0
        || stat.st_mode & 0o111 == 0
    {
        return Err(AdmissionError::Refused);
    }
    Ok(())
}

fn validate_elf_header(header: &[u8; 64]) -> Result<(), AdmissionError> {
    let machine = if cfg!(target_arch = "x86_64") {
        62_u16
    } else if cfg!(target_arch = "aarch64") {
        183_u16
    } else {
        return Err(AdmissionError::Refused);
    };
    let kind = u16::from_le_bytes([header[16], header[17]]);
    if header[..4] != *b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || header[6] != 1
        || u32::from_le_bytes([header[20], header[21], header[22], header[23]]) != 1
        || !matches!(kind, 2 | 3)
        || u16::from_le_bytes([header[18], header[19]]) != machine
    {
        return Err(AdmissionError::Refused);
    }
    Ok(())
}

fn is_normalized_absolute(path: &str) -> bool {
    path == "/"
        || (path.starts_with('/')
            && path.split('/').enumerate().all(|(index, component)| {
                index == 0 || (!component.is_empty() && component != "." && component != "..")
            }))
}
