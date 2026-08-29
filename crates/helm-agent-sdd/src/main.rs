use helm_agent_sdd::parse_json_value_without_duplicates;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};

struct Args {
    issue: u32,
    from: String,
    to: String,
}
struct Record {
    checkpoint: toml::Table,
    evidence: Vec<Value>,
    issue: u32,
    unknown_files: bool,
}
type TreeEntry = (Vec<u8>, Vec<u8>, Vec<u8>);
type Tree = BTreeMap<Vec<u8>, TreeEntry>;

fn emit(
    issue: Option<u32>,
    from: Option<&str>,
    to: Option<&str>,
    outcome: &str,
    obligations: &[(&str, &str)],
) {
    let q = |s: &str| serde_json::to_string(s).unwrap();
    let i = issue.map_or("null".into(), |n| n.to_string());
    let f = from.map_or("null".into(), q);
    let t = to.map_or("null".into(), q);
    let os = obligations
        .iter()
        .map(|(c, s)| format!("{{\"code\":{},\"status\":{}}}", q(c), q(s)))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"issue\":{i},\"from\":{f},\"to\":{t},\"outcome\":{},\"obligations\":[{os}]}}",
        q(outcome)
    );
}
fn failure(a: Option<&Args>, code: &str, outcome: &str, status: &str, exit: u8) -> ExitCode {
    if let Some(a) = a {
        emit(
            Some(a.issue),
            Some(&a.from),
            Some(&a.to),
            outcome,
            &[(code, status)],
        )
    } else {
        emit(None, None, None, outcome, &[(code, status)])
    };
    ExitCode::from(exit)
}
fn maturity(s: &str) -> bool {
    matches!(
        s,
        "probe" | "spike" | "prototype" | "candidate" | "production"
    )
}
fn args(v: &[String]) -> Option<Args> {
    let offset = match v.first()?.as_str() {
        "gate" => 0,
        "promote" if v.get(1)?.as_str() == "--dry-run" => 1,
        _ => return None,
    };
    if v.len() != 7 + offset
        || v.get(offset + 1)?.as_str() != "--issue"
        || v.get(offset + 3)?.as_str() != "--from"
        || v.get(offset + 5)?.as_str() != "--to"
    {
        return None;
    }
    let raw = &v[offset + 2];
    if raw.is_empty() || raw.starts_with('0') || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    };
    let issue = raw
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0 && *n <= 2_147_483_647)?;
    let from = v[offset + 4].clone();
    let to = v[offset + 6].clone();
    if !maturity(&from) || !maturity(&to) {
        return None;
    };
    Some(Args { issue, from, to })
}
fn load(a: &Args) -> Result<Record, &'static str> {
    let dir = Path::new(".agent/work").join(a.issue.to_string());
    if !dir.is_dir() {
        return Err("record");
    };
    let names = fs::read_dir(&dir)
        .map_err(|_| "record")?
        .map(|e| e.map(|x| x.file_name()))
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|_| "record")?;
    if !names.contains(std::ffi::OsStr::new("checkpoint.toml"))
        || !names.contains(std::ffi::OsStr::new("evidence.jsonl"))
    {
        return Err("schema");
    }
    let unknown_files = names.len() != 2;
    let checkpoint =
        String::from_utf8(fs::read(dir.join("checkpoint.toml")).map_err(|_| "record")?)
            .map_err(|_| "schema")?
            .parse()
            .map_err(|_| "schema")?;
    let text = String::from_utf8(fs::read(dir.join("evidence.jsonl")).map_err(|_| "record")?)
        .map_err(|_| "schema")?;
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("schema");
    };
    let mut evidence = vec![];
    for line in text.lines() {
        if line.is_empty() {
            return Err("schema");
        };
        evidence.push(parse_json_value_without_duplicates(line).map_err(|_| "schema")?)
    }
    Ok(Record {
        checkpoint,
        evidence,
        issue: a.issue,
        unknown_files,
    })
}
fn s<'a>(t: &'a toml::Table, k: &str) -> Option<&'a str> {
    t.get(k)?.as_str()
}
fn array<'a>(t: &'a toml::Table, k: &str) -> Option<&'a Vec<toml::Value>> {
    t.get(k)?.as_array()
}
fn obj(v: &Value) -> Option<&serde_json::Map<String, Value>> {
    v.as_object()
}
fn js<'a>(o: &'a serde_json::Map<String, Value>, k: &str) -> Option<&'a str> {
    o.get(k)?.as_str()
}
fn exact(t: &toml::Table, keys: &[&str]) -> bool {
    t.len() == keys.len() && keys.iter().all(|k| t.contains_key(*k))
}
fn id(s: &str) -> bool {
    let Some(n) = s.strip_prefix("ev-") else {
        return false;
    };
    !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n.bytes().any(|b| b != b'0')
}
fn sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f'))
}
fn timestamp(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, byte)| !matches!(i, 4 | 7 | 10 | 13 | 16 | 19) && !byte.is_ascii_digit())
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| {
        std::str::from_utf8(&bytes[range])
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0..4),
        number(5..7),
        number(8..10),
        number(11..13),
        number(14..16),
        number(17..19),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day) && hour <= 23 && minute <= 59 && second <= 59
}
fn maturity_record(s: &str) -> bool {
    matches!(s, "probe" | "spike" | "prototype")
}
fn strings(value: Option<&Vec<toml::Value>>, range: std::ops::RangeInclusive<usize>) -> bool {
    value.is_some_and(|values| {
        range.contains(&values.len()) && values.iter().all(toml::Value::is_str)
    })
}
fn json_exact(o: &serde_json::Map<String, Value>, required: &[&str], optional: &[&str]) -> bool {
    o.keys()
        .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
        && required.iter().all(|key| o.contains_key(*key))
}
fn schema(r: &Record) -> bool {
    let t = &r.checkpoint;
    let root = [
        "schema",
        "issue",
        "reason",
        "created_at",
        "current_maturity",
        "requested_maturity",
        "git_head",
        "question",
        "success_condition",
        "limitations",
        "affected_specs",
        "goal",
        "acceptance",
        "workspace",
        "next_actions",
        "completed",
        "hypotheses",
        "rejected_approaches",
    ];
    if t.keys().any(|k| !root.contains(&k.as_str()))
        || [
            "schema",
            "issue",
            "reason",
            "created_at",
            "current_maturity",
            "requested_maturity",
            "git_head",
            "question",
            "success_condition",
            "limitations",
            "affected_specs",
            "goal",
            "acceptance",
            "workspace",
            "next_actions",
        ]
        .iter()
        .any(|k| !t.contains_key(*k))
    {
        return false;
    }
    if s(t, "schema") != Some("helm-agent-sdd/checkpoint/v1")
        || t.get("issue")
            .and_then(toml::Value::as_integer)
            .filter(|n| *n >= 1 && *n <= 2_147_483_647)
            .is_none()
        || ["reason", "git_head", "question", "success_condition"]
            .iter()
            .any(|k| s(t, k).is_none())
        || !s(t, "created_at").is_some_and(timestamp)
        || !s(t, "current_maturity").is_some_and(maturity_record)
        || !s(t, "requested_maturity").is_some_and(maturity_record)
    {
        return false;
    }
    if !strings(array(t, "limitations"), 0..=16) || !strings(array(t, "affected_specs"), 0..=16) {
        return false;
    }
    let Some(goal) = t.get("goal").and_then(toml::Value::as_table) else {
        return false;
    };
    let Some(acc) = t.get("acceptance").and_then(toml::Value::as_table) else {
        return false;
    };
    let Some(ws) = t.get("workspace").and_then(toml::Value::as_table) else {
        return false;
    };
    let Some(next) = t.get("next_actions").and_then(toml::Value::as_table) else {
        return false;
    };
    if !exact(goal, &["statement"])
        || s(goal, "statement").is_none()
        || !exact(acc, &["criteria"])
        || !strings(array(acc, "criteria"), 1..=16)
        || !exact(ws, &["branch", "base", "dirty"])
        || s(ws, "branch").is_none()
        || s(ws, "base").is_none()
        || ws.get("dirty").and_then(toml::Value::as_bool).is_none()
        || !exact(next, &["items"])
        || !strings(array(next, "items"), 1..=3)
    {
        return false;
    }

    let mut checkpoint_evidence_refs = Vec::new();
    if let Some(completed) = t.get("completed") {
        let Some(completed) = completed.as_array() else {
            return false;
        };
        for value in completed {
            let Some(table) = value.as_table() else {
                return false;
            };
            if !exact(table, &["claim", "evidence"])
                || s(table, "claim").is_none()
                || !strings(array(table, "evidence"), 1..=16)
            {
                return false;
            }
            checkpoint_evidence_refs.extend(
                array(table, "evidence")
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap()),
            );
        }
    }
    let mut hypothesis_ids = HashSet::new();
    if let Some(hypotheses) = t.get("hypotheses") {
        let Some(hypotheses) = hypotheses.as_array() else {
            return false;
        };
        for value in hypotheses {
            let Some(table) = value.as_table() else {
                return false;
            };
            if !exact(table, &["id", "statement", "confidence"])
                || s(table, "id").is_none()
                || s(table, "statement").is_none()
                || !s(table, "confidence")
                    .is_some_and(|value| matches!(value, "low" | "medium" | "high"))
                || !hypothesis_ids.insert(s(table, "id").unwrap())
            {
                return false;
            }
        }
    }
    if let Some(rejected) = t.get("rejected_approaches") {
        let Some(rejected) = rejected.as_array() else {
            return false;
        };
        for value in rejected {
            let Some(table) = value.as_table() else {
                return false;
            };
            if !exact(table, &["approach", "reason", "evidence"])
                || s(table, "approach").is_none()
                || s(table, "reason").is_none()
                || !strings(array(table, "evidence"), 1..=16)
            {
                return false;
            }
            checkpoint_evidence_refs.extend(
                array(table, "evidence")
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap()),
            );
        }
    }

    let mut ids = HashSet::new();
    for v in &r.evidence {
        let Some(o) = obj(v) else { return false };
        let Some(kind) = js(o, "kind") else {
            return false;
        };
        let (required, optional): (&[&str], &[&str]) = match kind {
            "command" => (
                &[
                    "id",
                    "ts",
                    "kind",
                    "git_head",
                    "summary",
                    "command",
                    "exit_code",
                    "purpose",
                ],
                &["path"],
            ),
            "file-observation" => (
                &["id", "ts", "kind", "git_head", "path", "lines", "claim"],
                &["summary"],
            ),
            "decision" => (
                &[
                    "id",
                    "ts",
                    "kind",
                    "git_head",
                    "claim",
                    "reason",
                    "derived_from",
                ],
                &["summary"],
            ),
            _ => return false,
        };
        if !json_exact(o, required, optional)
            || !js(o, "id").is_some_and(id)
            || !js(o, "ts").is_some_and(timestamp)
            || js(o, "kind").is_none()
            || js(o, "git_head").is_none()
            || !ids.insert(js(o, "id").unwrap().to_owned())
        {
            return false;
        };
        match kind {
            "command" => {
                if ["summary", "command", "purpose"]
                    .iter()
                    .any(|key| js(o, key).is_none())
                    || o.get("path").is_some_and(|value| !value.is_string())
                    || !o
                        .get("exit_code")
                        .and_then(Value::as_i64)
                        .is_some_and(|n| (0..=255).contains(&n))
                {
                    return false;
                }
            }
            "file-observation" => {
                if ["path", "lines", "claim"]
                    .iter()
                    .any(|key| js(o, key).is_none())
                    || o.get("summary").is_some_and(|value| !value.is_string())
                {
                    return false;
                }
            }
            "decision" => {
                if ["claim", "reason"].iter().any(|key| js(o, key).is_none())
                    || o.get("summary").is_some_and(|value| !value.is_string())
                    || !o
                        .get("derived_from")
                        .and_then(Value::as_array)
                        .is_some_and(|values| {
                            (1..=16).contains(&values.len())
                                && values.iter().all(|value| value.as_str().is_some_and(id))
                        })
                {
                    return false;
                }
            }
            _ => unreachable!(),
        }
    }
    checkpoint_evidence_refs
        .into_iter()
        .all(|reference| id(reference) && ids.contains(reference))
}
fn printable(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}
fn prose(s: &str) -> bool {
    printable(s, 240)
        && !s.contains(|c| {
            matches!(
                c,
                '`' | '$'
                    | '{'
                    | '}'
                    | ';'
                    | '\\'
                    | '\"'
                    | '\''
                    | '<'
                    | '>'
                    | '|'
                    | '&'
                    | ':'
                    | '='
            )
        })
        && !s.contains("---")
}
fn command(s: &str) -> bool {
    printable(s, 512)
        && !s.starts_with('-')
        && !s.contains("..")
        && !s.contains('/')
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'_' | b'.' | b':' | b'@' | b'%' | b'+' | b',' | b' ' | b'-'
                )
        })
        && !s.split_whitespace().any(|x| {
            matches!(
                x,
                "fn" | "struct" | "enum" | "const" | "let" | "impl" | "use" | "pub" | "mod"
            )
        })
}
fn repository_path(s: &str) -> bool {
    printable(s, 240)
        && s.bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && s.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
        && s.split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
}
fn branch(s: &str) -> bool {
    printable(s, 128)
        && s.bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && s.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
        && !s.contains("..")
        && !s.ends_with(['.', '/'])
}
fn line_range(s: &str) -> bool {
    let positive = |value: &str| {
        !value.is_empty()
            && !value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit())
    };
    let less_or_equal = |start: &str, end: &str| {
        start.len() < end.len() || (start.len() == end.len() && start <= end)
    };
    match s.split_once('-') {
        Some((start, end)) => {
            positive(start) && positive(end) && !end.contains('-') && less_or_equal(start, end)
        }
        None => positive(s),
    }
}
fn bad(s: &str) -> bool {
    fn private_key(value: &str) -> bool {
        let prefix = "-----BEGIN ";
        let suffix = "PRIVATE KEY-----";
        let mut rest = value;
        while let Some(start) = rest.find(prefix) {
            let after = &rest[start + prefix.len()..];
            if after.starts_with(suffix) {
                return true;
            }
            if let Some(end) = after.find(suffix) {
                let qualifier = &after[..end];
                if qualifier.len() >= 2
                    && qualifier.ends_with(' ')
                    && qualifier.bytes().all(|byte| {
                        byte == b' ' || byte.is_ascii_uppercase() || byte.is_ascii_digit()
                    })
                {
                    return true;
                }
            }
            rest = &after[after.len().min(1)..];
        }
        false
    }
    fn jwt(value: &str) -> bool {
        let bytes = value.as_bytes();
        let token = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-');
        for start in 0..bytes.len() {
            if (start > 0 && token(bytes[start - 1])) || !token(bytes[start]) {
                continue;
            }
            let mut cursor = start;
            for segment in 0..3 {
                let segment_start = cursor;
                while cursor < bytes.len() && token(bytes[cursor]) {
                    cursor += 1;
                }
                if cursor - segment_start < 10 {
                    break;
                }
                if segment < 2 {
                    if bytes.get(cursor) != Some(&b'.') {
                        break;
                    }
                    cursor += 1;
                } else if cursor == bytes.len() || !token(bytes[cursor]) {
                    return true;
                }
            }
        }
        false
    }
    fn assignment(value: &str) -> bool {
        let lower = value.to_ascii_lowercase();
        let keys = [
            "authorization",
            "bearer",
            "api_key",
            "api-key",
            "apikey",
            "access_token",
            "access-token",
            "accesstoken",
            "secret",
            "password",
            "passwd",
            "private_key",
            "private-key",
            "privatekey",
            "session",
            "cookie",
        ];
        keys.iter().any(|key| {
            lower.match_indices(key).any(|(start, _)| {
                let bytes = lower.as_bytes();
                let mut cursor = start + key.len();
                while bytes.get(cursor) == Some(&b' ') {
                    cursor += 1;
                }
                if !matches!(bytes.get(cursor), Some(b'=') | Some(b':')) {
                    return false;
                }
                cursor += 1;
                while bytes.get(cursor) == Some(&b' ') {
                    cursor += 1;
                }
                bytes.get(cursor).is_some_and(|byte| *byte != b' ')
            })
        })
    }
    private_key(s) || jwt(s) || assignment(s)
}
fn hygiene(r: &Record) -> bool {
    if r.unknown_files {
        return false;
    }
    let t = &r.checkpoint;
    let goal = t.get("goal").unwrap().as_table().unwrap();
    let acceptance = t.get("acceptance").unwrap().as_table().unwrap();
    let workspace = t.get("workspace").unwrap().as_table().unwrap();
    let next_actions = t.get("next_actions").unwrap().as_table().unwrap();
    if ["reason", "question", "success_condition"]
        .iter()
        .any(|k| !prose(s(t, k).unwrap()))
        || array(t, "limitations")
            .unwrap()
            .iter()
            .any(|value| !prose(value.as_str().unwrap()))
        || !prose(s(goal, "statement").unwrap())
        || array(acceptance, "criteria").unwrap().iter().any(|value| {
            let value = value.as_str().unwrap();
            document_reference(value).is_none() && !prose(value)
        })
        || !branch(s(workspace, "branch").unwrap())
        || array(next_actions, "items")
            .unwrap()
            .iter()
            .any(|value| !prose(value.as_str().unwrap()))
    {
        return false;
    };
    for completed in t
        .get("completed")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        if !prose(s(completed.as_table().unwrap(), "claim").unwrap()) {
            return false;
        }
    }
    let mut hypothesis_ids = HashSet::new();
    for hypothesis in t
        .get("hypotheses")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let hypothesis = hypothesis.as_table().unwrap();
        if !prose(s(hypothesis, "id").unwrap())
            || !prose(s(hypothesis, "statement").unwrap())
            || !hypothesis_ids.insert(s(hypothesis, "id").unwrap())
        {
            return false;
        }
    }
    for rejected in t
        .get("rejected_approaches")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let rejected = rejected.as_table().unwrap();
        if !prose(s(rejected, "approach").unwrap()) || !prose(s(rejected, "reason").unwrap()) {
            return false;
        }
    }
    let mut strings = vec![];
    fn walk<'a>(v: &'a toml::Value, o: &mut Vec<&'a str>) {
        match v {
            toml::Value::String(x) => o.push(x),
            toml::Value::Array(x) => {
                for x in x {
                    walk(x, o)
                }
            }
            toml::Value::Table(x) => {
                for x in x.values() {
                    walk(x, o)
                }
            }
            _ => {}
        }
    }
    for v in t.values() {
        walk(v, &mut strings)
    }
    for e in &r.evidence {
        let o = e.as_object().unwrap();
        fn walk_json<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
            match value {
                Value::String(value) => out.push(value),
                Value::Array(values) => {
                    for value in values {
                        walk_json(value, out);
                    }
                }
                Value::Object(values) => {
                    for value in values.values() {
                        walk_json(value, out);
                    }
                }
                _ => {}
            }
        }
        walk_json(e, &mut strings);
        let summary_invalid = o
            .get("summary")
            .is_some_and(|value| !prose(value.as_str().unwrap()));
        let fields_invalid = match js(o, "kind").unwrap() {
            "command" => {
                !prose(js(o, "summary").unwrap())
                    || !command(js(o, "command").unwrap())
                    || !prose(js(o, "purpose").unwrap())
                    || o.get("path")
                        .is_some_and(|value| !repository_path(value.as_str().unwrap()))
            }
            "file-observation" => {
                !repository_path(js(o, "path").unwrap())
                    || !line_range(js(o, "lines").unwrap())
                    || !prose(js(o, "claim").unwrap())
            }
            "decision" => !prose(js(o, "claim").unwrap()) || !prose(js(o, "reason").unwrap()),
            _ => unreachable!(),
        };
        if summary_invalid || fields_invalid {
            return false;
        }
    }
    strings
        .into_iter()
        .all(|value| printable(value, usize::MAX) && !bad(value))
}
fn git_commit(id: &str) -> bool {
    if !sha(id) {
        return false;
    }
    Command::new("git")
        .args(["cat-file", "-e", &format!("{id}^{{commit}}")])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn decision_provenance(r: &Record) -> bool {
    let evidence: Vec<_> = r.evidence.iter().map(|value| obj(value).unwrap()).collect();
    let by_id: BTreeMap<_, _> = evidence
        .iter()
        .enumerate()
        .map(|(index, item)| (js(item, "id").unwrap(), index))
        .collect();
    evidence.iter().enumerate().all(|(index, item)| {
        js(item, "kind") != Some("decision")
            || item
                .get("derived_from")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .all(|reference| {
                    let Some(parent_index) = by_id.get(reference.as_str().unwrap()).copied() else {
                        return false;
                    };
                    parent_index < index
                        && js(evidence[parent_index], "kind") != Some("decision")
                        && js(evidence[parent_index], "git_head") == js(item, "git_head")
                })
    })
}

fn document_reference(value: &str) -> Option<(&str, &str)> {
    let (kind, number) = value.split_once('-')?;
    (matches!(kind, "ADR" | "SPEC")
        && number.len() == 4
        && number.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some((kind, number))
}

fn accepted_status(line: &str) -> bool {
    const PLAIN: &str = "- **Status:** Accepted";
    if line == PLAIN {
        return true;
    }
    let Some(date) = line
        .strip_prefix("- **Status:** Accepted (")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    date.len() == 10
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn accepted_reference(reference: &str) -> bool {
    let Some((kind, number)) = document_reference(reference) else {
        return false;
    };
    let directory = if kind == "ADR" {
        "docs/adr"
    } else {
        "docs/specs"
    };
    let directory_prefix = format!("{directory}/");
    let basename_prefix = format!("{number}-");
    let Some(paths) = git_output(&["ls-tree", "-rz", "--name-only", "HEAD", "--", directory])
    else {
        return false;
    };
    let matches: Vec<_> = paths
        .split(|byte| *byte == 0)
        .filter_map(|path| std::str::from_utf8(path).ok())
        .filter(|path| {
            path.strip_prefix(&directory_prefix)
                .is_some_and(|basename| {
                    !basename.contains('/')
                        && basename.starts_with(&basename_prefix)
                        && basename.ends_with(".md")
                        && basename.len() > basename_prefix.len() + ".md".len()
                })
        })
        .collect();
    let [path] = matches.as_slice() else {
        return false;
    };
    let Some(content) = git_output(&["show", &format!("HEAD:{path}")]) else {
        return false;
    };
    let Ok(content) = std::str::from_utf8(&content) else {
        return false;
    };
    let expected_title = format!("# {kind} {number} — ");
    content
        .split('\n')
        .next()
        .is_some_and(|line| line.starts_with(&expected_title) && line.len() > expected_title.len())
        && content
            .split('\n')
            .filter(|line| accepted_status(line))
            .count()
            == 1
}

fn accepted_refs(r: &Record) -> bool {
    let affected = array(&r.checkpoint, "affected_specs").unwrap();
    if !affected
        .iter()
        .all(|value| accepted_reference(value.as_str().unwrap()))
    {
        return false;
    }
    let acceptance = r
        .checkpoint
        .get("acceptance")
        .and_then(toml::Value::as_table)
        .unwrap();
    array(acceptance, "criteria").unwrap().iter().all(|value| {
        let value = value.as_str().unwrap();
        document_reference(value).is_none() || accepted_reference(value)
    })
}

fn git_output(args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then_some(output.stdout)
}

fn parent_commit() -> Option<String> {
    let output = git_output(&["rev-list", "--parents", "-n", "1", "HEAD"])?;
    let line = std::str::from_utf8(&output).ok()?.trim();
    let fields: Vec<_> = line.split_ascii_whitespace().collect();
    (fields.len() == 2).then(|| fields[1].to_owned())
}

fn tree(commit: &str) -> Option<Tree> {
    let output = git_output(&["ls-tree", "-rz", "--full-tree", commit])?;
    let mut entries = BTreeMap::new();
    for raw in output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let tab = raw.iter().position(|byte| *byte == b'\t')?;
        let mut metadata = raw[..tab].split(|byte| *byte == b' ');
        let mode = metadata.next()?.to_vec();
        let kind = metadata.next()?.to_vec();
        let object = metadata.next()?.to_vec();
        if metadata.next().is_some() {
            return None;
        }
        entries.insert(raw[tab + 1..].to_vec(), (mode, kind, object));
    }
    Some(entries)
}

fn carrier(issue: u32) -> (Option<String>, bool) {
    let Some(parent) = parent_commit() else {
        return (None, false);
    };
    let Some(before) = tree(&parent) else {
        return (Some(parent), false);
    };
    let Some(after) = tree("HEAD") else {
        return (Some(parent), false);
    };
    let checkpoint = format!(".agent/work/{issue}/checkpoint.toml").into_bytes();
    let evidence = format!(".agent/work/{issue}/evidence.jsonl").into_bytes();
    let allowed = [&checkpoint, &evidence];
    let changed: HashSet<_> = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect();
    let exact_paths = changed.len() == 2 && allowed.iter().all(|path| changed.contains(*path));
    let regular = allowed.iter().all(|path| {
        after
            .get(*path)
            .is_some_and(|(mode, kind, _)| mode == b"100644" && kind == b"blob")
            && before
                .get(*path)
                .is_none_or(|(mode, kind, _)| mode == b"100644" && kind == b"blob")
    });
    (Some(parent), exact_paths && regular)
}

fn ancestor(base: &str, commit: &str) -> bool {
    Command::new("git")
        .args(["merge-base", "--is-ancestor", base, commit])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn clean() -> bool {
    git_output(&[
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
    ])
    .is_some_and(|output| output.is_empty())
}

fn main() -> ExitCode {
    let raw: Vec<_> = env::args().skip(1).collect();
    let Some(a) = args(&raw) else {
        return failure(None, "cli", "invalid", "invalid", 3);
    };
    let r = match load(&a) {
        Ok(r) => r,
        Err("record") => return failure(Some(&a), "record", "invalid", "invalid", 3),
        Err(_) => return failure(Some(&a), "schema", "invalid", "invalid", 3),
    };
    if !schema(&r) {
        return failure(Some(&a), "schema", "invalid", "invalid", 3);
    };
    if !hygiene(&r) {
        return failure(Some(&a), "hygiene", "invalid", "invalid", 3);
    };
    if !git_commit(s(&r.checkpoint, "git_head").unwrap())
        || !git_commit(
            s(
                r.checkpoint
                    .get("workspace")
                    .and_then(toml::Value::as_table)
                    .unwrap(),
                "base",
            )
            .unwrap(),
        )
        || !r
            .evidence
            .iter()
            .all(|e| git_commit(js(obj(e).unwrap(), "git_head").unwrap()))
    {
        return failure(Some(&a), "git_objects", "invalid", "invalid", 3);
    };
    if r.checkpoint.get("issue").and_then(toml::Value::as_integer) != Some(r.issue as i64) {
        return failure(Some(&a), "issue_directory", "invalid", "invalid", 3);
    };
    if !decision_provenance(&r) {
        return failure(Some(&a), "decision_provenance", "invalid", "invalid", 3);
    }
    if !accepted_refs(&r) {
        return failure(Some(&a), "accepted_refs", "invalid", "invalid", 3);
    }
    if matches!(a.to.as_str(), "candidate" | "production") {
        emit(
            Some(a.issue),
            Some(&a.from),
            Some(&a.to),
            "unsupported",
            &[
                ("accepted_refs", "met"),
                ("decision_provenance", "met"),
                ("git_objects", "met"),
                ("hygiene", "met"),
                ("issue_directory", "met"),
                ("schema", "met"),
                ("transition", "unsupported"),
            ],
        );
        return ExitCode::from(4);
    };
    if a.from != s(&r.checkpoint, "current_maturity").unwrap()
        || a.to != s(&r.checkpoint, "requested_maturity").unwrap()
        || !matches!(
            (a.from.as_str(), a.to.as_str()),
            ("probe", "spike") | ("spike", "prototype")
        )
    {
        emit(
            Some(a.issue),
            Some(&a.from),
            Some(&a.to),
            "invalid",
            &[
                ("accepted_refs", "met"),
                ("decision_provenance", "met"),
                ("git_objects", "met"),
                ("hygiene", "met"),
                ("issue_directory", "met"),
                ("schema", "met"),
                ("transition", "invalid"),
            ],
        );
        return ExitCode::from(3);
    };
    let (parent, is_carrier) = carrier(a.issue);
    let checkpoint_head = s(&r.checkpoint, "git_head").unwrap();
    let workspace = r
        .checkpoint
        .get("workspace")
        .and_then(toml::Value::as_table)
        .unwrap();
    let workspace_base = s(workspace, "base").unwrap();
    let workspace_clean =
        workspace.get("dirty").and_then(toml::Value::as_bool) == Some(false) && clean();
    let fresh_checkpoint = parent.as_deref().is_some_and(|parent| {
        is_carrier && checkpoint_head == parent && ancestor(workspace_base, parent)
    });
    let fresh_evidence = parent.as_deref().is_some_and(|parent| {
        is_carrier
            && r.evidence
                .iter()
                .all(|e| js(obj(e).unwrap(), "git_head") == Some(parent))
    });
    let transition_evidence = parent.as_deref().is_some_and(|parent| {
        r.evidence.iter().any(|e| {
            let o = obj(e).unwrap();
            js(o, "git_head") == Some(parent)
                && (js(o, "kind") == Some("file-observation")
                    || (js(o, "kind") == Some("command")
                        && o.get("exit_code").and_then(Value::as_i64) == Some(0)))
        })
    });
    let transition_fields = if a.from == "probe" {
        true
    } else {
        array(&r.checkpoint, "limitations").is_some_and(|values| !values.is_empty())
    };
    let obligations = [
        ("accepted_refs", "met"),
        (
            "clean_workspace",
            if workspace_clean { "met" } else { "unmet" },
        ),
        ("decision_provenance", "met"),
        (
            "fresh_checkpoint",
            if fresh_checkpoint { "met" } else { "unmet" },
        ),
        (
            "fresh_evidence",
            if fresh_evidence { "met" } else { "unmet" },
        ),
        ("git_objects", "met"),
        ("hygiene", "met"),
        ("issue_directory", "met"),
        ("schema", "met"),
        ("transition", "met"),
        (
            "transition_evidence",
            if transition_evidence { "met" } else { "unmet" },
        ),
        (
            "transition_fields",
            if transition_fields { "met" } else { "unmet" },
        ),
    ];
    let pass = obligations.iter().all(|(_, status)| *status == "met");
    emit(
        Some(a.issue),
        Some(&a.from),
        Some(&a.to),
        if pass { "pass" } else { "unmet" },
        &obligations,
    );
    ExitCode::from(if pass { 0 } else { 2 })
}
