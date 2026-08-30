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
    use super::{parse_exec, DesktopExecError};

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
}
