//! Shareable-text sanitizer (`F-148`).
//!
//! Behavior port of `log_sanitizer.py`: constant tables and phase order are
//! lifted verbatim (read-only reference, never modified). Pure functions,
//! no filesystem access, infallible by construction (hand scanners + regexes
//! compiled once — nothing here can fail at runtime).

use std::sync::LazyLock;

pub const REDACTED_HOME: &str = "<HOME>";
pub const REDACTED_USER: &str = "<USER>";
pub const REDACTED_PATH: &str = "<REDACTED:path>";
pub const REDACTED_IP: &str = "<REDACTED:ip>";
pub const REDACTED_EMAIL: &str = "<REDACTED:email>";
pub const REDACTED_CREDENTIAL: &str = "<REDACTED:credential>";
pub const REDACTED_LICENSE_FIELD: &str = "<REDACTED:license>";
pub const REDACTED_FINGERPRINT: &str = "<REDACTED:fingerprint>";
pub const REDACTED_ACTIVATION_CODE: &str = "<REDACTED:activation-code>";

const SENSITIVE_FIELDS: &[(&str, &str)] = &[
    ("account", REDACTED_USER),
    ("account_name", REDACTED_USER),
    ("activation_code", REDACTED_ACTIVATION_CODE),
    ("activation_request_code", REDACTED_ACTIVATION_CODE),
    ("email", REDACTED_EMAIL),
    ("email_address", REDACTED_EMAIL),
    ("license_holder", REDACTED_LICENSE_FIELD),
    ("license_id", REDACTED_LICENSE_FIELD),
    ("license_signature", REDACTED_LICENSE_FIELD),
    ("machine_fingerprint", REDACTED_FINGERPRINT),
    ("machine_fingerprint_hash", REDACTED_FINGERPRINT),
    ("player", REDACTED_USER),
    ("player_name", REDACTED_USER),
    ("username", REDACTED_USER),
    ("user_name", REDACTED_USER),
];

const CREDENTIAL_KEYS: &[&str] = &[
    "api_key",
    "authorization",
    "auth_header",
    "client_secret",
    "cookie",
    "jwt",
    "password",
    "passphrase",
    "pin",
    "private_key",
    "proxy_authorization",
    "refresh_token",
    "session_id",
    "session_token",
    "set_cookie",
    "signing_key",
    "token",
    "access_token",
    "webhook_secret",
];

const WHOLE_VALUE_KEYS: &[&str] = &[
    "authorization",
    "auth_header",
    "cookie",
    "private_key",
    "proxy_authorization",
    "set_cookie",
    "signing_key",
];

const CREDENTIAL_SUFFIXES: &[&str] = &[
    "_access_key",
    "_access_token",
    "_api_key",
    "_client_secret",
    "_passphrase",
    "_password",
    "_private_key",
    "_refresh_token",
    "_session_token",
    "_signing_key",
    "_webhook_secret",
];

/// Normalize a key exactly like the old `_normalize_key`.
fn normalize_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut last_underscore = true;
    for c in key.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn is_credential_key(normalized: &str) -> bool {
    CREDENTIAL_KEYS.contains(&normalized)
        || CREDENTIAL_SUFFIXES.iter().any(|s| normalized.ends_with(s))
}

fn replacement_for_key(key: &str) -> Option<&'static str> {
    let n = normalize_key(key);
    if is_credential_key(&n) {
        return Some(REDACTED_CREDENTIAL);
    }
    SENSITIVE_FIELDS
        .iter()
        .find(|(k, _)| *k == n)
        .map(|(_, v)| *v)
}

fn is_path_field(key: &str) -> bool {
    let n = normalize_key(key);
    matches!(
        n.as_str(),
        "database"
            | "destination"
            | "file_path"
            | "license_path"
            | "path"
            | "source_path"
            | "target"
            | "target_path"
    ) || ["_dir", "_directory", "_folder", "_path", "_root"]
        .iter()
        .any(|s| n.ends_with(s))
}

macro_rules! re {
    ($pat:literal) => {{
        static RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new($pat).expect("static pattern"));
        &*RE
    }};
}

/// Absolute path → `<REDACTED:path>/basename`, style-preserved.
fn redacted_path_with_basename(value: &str) -> String {
    let windows_style = matches!(value.as_bytes(), [a, b':', c, ..] if a.is_ascii_alphabetic() && (*c == b'\\' || *c == b'/'))
        || value.starts_with("\\\\");
    let stripped = value.trim_end_matches(['\\', '/']);
    let basename = stripped.rsplit(['\\', '/']).next().unwrap_or("");
    if basename.is_empty() || matches!(basename, "." | "\\" | "/") {
        return REDACTED_PATH.to_string();
    }
    let sep = if windows_style { "\\" } else { "/" };
    format!("{REDACTED_PATH}{sep}{basename}")
}

fn is_absolute_path_text(value: &str) -> bool {
    matches!(value.as_bytes(), [a, b':', c, ..] if a.is_ascii_alphabetic() && (*c == b'\\' || *c == b'/'))
        || value.starts_with("\\\\")
        || value.starts_with('/')
}

/// Default homes/users from the environment (tests pass explicit lists).
fn default_homes_users() -> (Vec<String>, Vec<String>) {
    let mut homes = Vec::new();
    for key in ["HOME", "USERPROFILE"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() && !homes.contains(&v) {
                homes.push(v);
            }
        }
    }
    homes.sort_by_key(|h| std::cmp::Reverse(h.len()));
    let mut users = Vec::new();
    for key in ["USERNAME", "USER"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() && !users.contains(&v) {
                users.push(v);
            }
        }
    }
    (homes, users)
}

/// Shareable text with identity and secrets removed (F-148).
pub fn sanitize(input: &str) -> String {
    let (homes, users) = default_homes_users();
    sanitize_with(input, &homes, &users)
}

pub fn sanitize_with(input: &str, home_paths: &[String], usernames: &[String]) -> String {
    // Structured JSON first (whole-document branch).
    if let Some(out) = try_json(input, home_paths, usernames) {
        return out;
    }
    sanitize_plain(input, home_paths, usernames)
}

fn try_json(input: &str, homes: &[String], users: &[String]) -> Option<String> {
    let stripped = input.trim_start();
    if !stripped.starts_with('{') && !stripped.starts_with('[') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    if !value.is_object() && !value.is_array() {
        return None;
    }
    let safe = sanitize_json_value(&value, homes, users, "");
    // Keys sorted like the old `sort_keys=True`.
    let sorted = sort_json(&safe);
    serde_json::to_string(&sorted).ok()
}

fn sort_json(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::with_capacity(m.len());
            for k in keys {
                out.insert(k.clone(), sort_json(&m[k]));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(a) => serde_json::Value::Array(a.iter().map(sort_json).collect()),
        _ => v.clone(),
    }
}

fn sanitize_json_value(
    value: &serde_json::Value,
    homes: &[String],
    users: &[String],
    key: &str,
) -> serde_json::Value {
    use serde_json::Value as V;
    if !key.is_empty() {
        if let Some(r) = replacement_for_key(key) {
            return V::String(r.to_string());
        }
    }
    match value {
        V::Object(m) => V::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), sanitize_json_value(v, homes, users, k)))
                .collect(),
        ),
        V::Array(a) => V::Array(
            a.iter()
                .map(|v| sanitize_json_value(v, homes, users, ""))
                .collect(),
        ),
        V::String(s) => {
            let mut safe = sanitize_scalar(s, homes, users);
            if !key.is_empty() && is_path_field(key) {
                if let Some(r) = redact_absolute_path_value(&safe) {
                    safe = r;
                }
            }
            V::String(safe)
        }
        _ => value.clone(),
    }
}

fn redact_absolute_path_value(value: &str) -> Option<String> {
    let leading_len = value.len() - value.trim_start().len();
    let trailing_len = value.len() - value.trim_end().len();
    let (leading, rest) = value.split_at(leading_len);
    let (core, trailing) = rest.split_at(rest.len() - trailing_len);
    let (quote, core) = match core.as_bytes() {
        [q, .., l] if core.len() >= 2 && q == l && (*q == b'\'' || *q == b'"') => {
            (core[..1].to_string(), &core[1..core.len() - 1])
        }
        _ => (String::new(), core),
    };
    if !is_absolute_path_text(core) {
        return None;
    }
    Some(format!(
        "{leading}{quote}{}{quote}{trailing}",
        redacted_path_with_basename(core)
    ))
}

fn sanitize_plain(input: &str, homes: &[String], users: &[String]) -> String {
    let mut safe = re!(r"MBD-ACTREQ-v1\.[A-Za-z0-9_-]+")
        .replace_all(input, REDACTED_ACTIVATION_CODE)
        .into_owned();
    safe = re!(r"(?is)-----BEGIN [^-\r\n]*PRIVATE KEY-----.*?-----END [^-\r\n]*PRIVATE KEY-----")
        .replace_all(&safe, REDACTED_CREDENTIAL)
        .into_owned();
    safe = redact_home_paths(&safe, homes);
    safe = redact_user_segments(&safe, users);
    safe = sanitize_field_lines(&safe);
    safe = sanitize_quoted_key_values(&safe);
    safe = sanitize_inline_credentials(&safe);
    safe = sanitize_cli_credentials(&safe);
    safe = sanitize_bearer_and_jwt(&safe);
    safe = sanitize_urls(&safe);
    safe = redact_ip_addresses(&safe);
    safe = redact_quoted_absolute_paths(&safe);
    // Email: Rust regex has no look-behind; capture the boundary instead.
    let email = re!(
        r"(^|[^A-Za-z0-9.!#$%&'*+/=?^_`{|}~-])([A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)"
    );
    email
        .replace_all(&safe, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], REDACTED_EMAIL)
        })
        .into_owned()
}

fn sanitize_scalar(input: &str, homes: &[String], users: &[String]) -> String {
    // JSON scalar branch: same passes minus field-line handling.
    let mut safe = re!(r"MBD-ACTREQ-v1\.[A-Za-z0-9_-]+")
        .replace_all(input, REDACTED_ACTIVATION_CODE)
        .into_owned();
    safe = re!(r"(?is)-----BEGIN [^-\r\n]*PRIVATE KEY-----.*?-----END [^-\r\n]*PRIVATE KEY-----")
        .replace_all(&safe, REDACTED_CREDENTIAL)
        .into_owned();
    safe = redact_home_paths(&safe, homes);
    safe = redact_user_segments(&safe, users);
    safe = sanitize_inline_credentials(&safe);
    safe = sanitize_cli_credentials(&safe);
    safe = sanitize_bearer_and_jwt(&safe);
    safe = sanitize_urls(&safe);
    safe = redact_ip_addresses(&safe);
    safe = redact_quoted_absolute_paths(&safe);
    safe
}

fn redact_home_paths(input: &str, homes: &[String]) -> String {
    let mut variants: Vec<(String, bool)> = Vec::new();
    for home in homes {
        variants.push((home.clone(), false));
        variants.push((home.replace('\\', "/"), false));
    }
    variants.sort_by_key(|(h, _)| std::cmp::Reverse(h.len()));
    variants.dedup();
    let mut safe = input.to_string();
    for (home, _) in variants {
        if home.len() < 3 {
            continue;
        }
        if matches!(home.as_bytes(), [a, b':', _, ..] if a.is_ascii_alphabetic()) {
            let pat = regex::Regex::new(&format!("(?i){}", regex::escape(&home)))
                .expect("static pattern");
            safe = pat.replace_all(&safe, REDACTED_HOME).into_owned();
        } else {
            safe = safe.replace(&home, REDACTED_HOME);
        }
    }
    safe
}

fn redact_user_segments(input: &str, users: &[String]) -> String {
    let mut safe = input.to_string();
    // Specific known usernames first (longest context), then the generic
    // any-name-under-Users fallback. Consume/re-emit the trailing slash:
    // Rust regex has no look-ahead.
    let mut patterns: Vec<String> = Vec::new();
    for user in users {
        if user.len() < 3 {
            continue;
        }
        let esc = regex::escape(user);
        patterns.push(format!(r"(?i)([\\/](?:Users|home)[\\/]){esc}([\\/])"));
        patterns.push(format!(
            r"(?i)([\\/]Documents and Settings[\\/]){esc}([\\/])"
        ));
    }
    patterns.push(r"(?i)([\\/](?:Users|home)[\\/])([^\\/\s<>]+)([\\/])".to_string());
    patterns.push(r"(?i)([\\/]Documents and Settings[\\/])([^\\/\s<>]+)([\\/])".to_string());
    for pat in patterns {
        let r = re_raw(&pat);
        safe = r
            .replace_all(&safe, |caps: &regex::Captures| {
                format!("{}{}{}", &caps[1], REDACTED_USER, &caps[3])
            })
            .into_owned();
    }
    safe
}

fn re_raw(pat: &str) -> regex::Regex {
    regex::Regex::new(pat).expect("static pattern")
}

fn sanitize_field_lines(input: &str) -> String {
    let line_re = re!(
        r#"^(?P<prefix>\s*["']?)(?P<key>[A-Za-z0-9_. -]+)(?P<suffix>["']?\s*)(?P<separator>[:=])(?P<spacing>\s*)(?P<value>.*)$"#
    );
    let mut out = String::with_capacity(input.len());
    for piece in input.split_inclusive('\n') {
        let (body, newline) = split_newline(piece);
        let Some(caps) = line_re.captures(body) else {
            out.push_str(piece);
            continue;
        };
        let key = &caps["key"];
        let normalized = normalize_key(key);
        let replacement = if is_credential_key(&normalized) {
            Some(redact_credential_field_value(&normalized, &caps["value"]))
        } else {
            replacement_for_key(key).map(|s| s.to_string())
        };
        let replacement = match replacement {
            Some(r) => r,
            None => match redact_absolute_path_value(&caps["value"]) {
                Some(r) if is_path_field(key) => r,
                _ => {
                    out.push_str(piece);
                    continue;
                }
            },
        };
        out.push_str(&caps["prefix"]);
        out.push_str(key);
        out.push_str(&caps["suffix"]);
        out.push_str(&caps["separator"]);
        out.push_str(&caps["spacing"]);
        out.push_str(&replacement);
        out.push_str(newline);
    }
    out
}

fn split_newline(line: &str) -> (&str, &str) {
    if let Some(b) = line.strip_suffix("\r\n") {
        (b, "\r\n")
    } else if let Some(b) = line.strip_suffix(['\r', '\n']) {
        (b, &line[line.len() - 1..])
    } else {
        (line, "")
    }
}

fn redact_credential_field_value(key: &str, value: &str) -> String {
    if WHOLE_VALUE_KEYS.contains(&key) {
        return REDACTED_CREDENTIAL.to_string();
    }
    // First token redacted, rest (e.g. "; Path=/") preserved.
    let trimmed = value.trim_start();
    let leading = &value[..value.len() - trimmed.len()];
    let end = trimmed
        .find(|c: char| c.is_whitespace() || c == ';' || c == ',')
        .unwrap_or(trimmed.len());
    // Keep surrounding quotes out of the redaction like the source: the whole
    // first token (quotes included) is replaced.
    let rest = &trimmed[end..];
    format!("{leading}{REDACTED_CREDENTIAL}{rest}")
}

fn sanitize_quoted_key_values(input: &str) -> String {
    // No backreferences in Rust regex: match both quotes, compare in code.
    let r = re!(
        r#"(?P<prefix>["'](?P<key>[A-Za-z0-9_. -]+)["']\s*:\s*)(?P<open>["'])(?P<value>.*?)(?P<close>["'])"#
    );
    r.replace_all(input, |caps: &regex::Captures| {
        if caps["open"] != caps["close"] {
            return caps[0].to_string();
        }
        match replacement_for_key(&caps["key"]) {
            Some(rep) => format!("{}{}{}", &caps["prefix"], rep, &caps["open"]),
            None => caps[0].to_string(),
        }
    })
    .into_owned()
}

fn sanitize_inline_credentials(input: &str) -> String {
    // `(?<![A-Za-z0-9_.-])` has no Rust equivalent: capture the boundary.
    let r = re!(
        r#"(?i)(?P<prefix>(?:^|[^A-Za-z0-9_.-])(?P<key>[A-Za-z][A-Za-z0-9_.-]*)\s*[:=]\s*)(?P<value>"[^"]*"|'[^']*'|[^\s,;&#]+)"#
    );
    r.replace_all(input, |caps: &regex::Captures| {
        if !is_credential_key(&normalize_key(&caps["key"])) {
            return caps[0].to_string();
        }
        format!("{}{}", &caps["prefix"], REDACTED_CREDENTIAL)
    })
    .into_owned()
}

fn sanitize_cli_credentials(input: &str) -> String {
    let cli = re!(
        r#"(?i)(?P<prefix>--(?P<key>[A-Za-z][A-Za-z0-9_.-]*)(?P<separator>=|\s+))(?P<value>"[^"]*"|'[^']*'|[^\s]+)"#
    );
    let out = cli
        .replace_all(input, |caps: &regex::Captures| {
            if !is_credential_key(&normalize_key(&caps["key"])) {
                return caps[0].to_string();
            }
            format!("{}{}", &caps["prefix"], REDACTED_CREDENTIAL)
        })
        .into_owned();
    let jvm = re!(r"(?i)(?P<prefix>-D(?P<key>[A-Za-z][A-Za-z0-9_.-]*)\s*=\s*)(?P<value>[^\s]+)");
    jvm.replace_all(&out, |caps: &regex::Captures| {
        if !is_credential_key(&normalize_key(&caps["key"])) {
            return caps[0].to_string();
        }
        format!("{}{}", &caps["prefix"], REDACTED_CREDENTIAL)
    })
    .into_owned()
}

fn sanitize_bearer_and_jwt(input: &str) -> String {
    let auth = re!(r"(?i)(?P<prefix>\b(?:Bearer|Basic)\s+)(?P<value>[A-Za-z0-9._~+/=-]+)");
    let out = auth
        .replace_all(input, |caps: &regex::Captures| {
            format!("{}{}", &caps["prefix"], REDACTED_CREDENTIAL)
        })
        .into_owned();
    re!(r"\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
        .replace_all(&out, REDACTED_CREDENTIAL)
        .into_owned()
}

fn sanitize_urls(input: &str) -> String {
    let url_re = re!(r#"(?i)https?://(?:(?:<REDACTED:[^>\r\n]+>)|[^\s<>"'])+"#);
    url_re
        .replace_all(input, |caps: &regex::Captures| sanitize_one_url(&caps[0]))
        .into_owned()
}

fn sanitize_one_url(raw: &str) -> String {
    let mut trailing = String::new();
    let mut url = raw.to_string();
    while url.ends_with([')', ',', '.', ';']) {
        trailing.insert(0, url.pop().unwrap_or(')'));
    }
    // scheme://authority | path?query#fragment — hand-split, no url crate here.
    let after_scheme = match url.find("://") {
        Some(i) => i + 3,
        None => return raw.to_string(),
    };
    let (scheme, rest) = url.split_at(after_scheme - 3);
    let rest = &rest[3..];
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path_query) = rest.split_at(auth_end);
    let host = match authority.rfind('@') {
        Some(i) => format!("{REDACTED_CREDENTIAL}@{}", &authority[i + 1..]),
        None => authority.to_string(),
    };
    // Split path?query#fragment.
    let (path_q, fragment) = match path_query.find('#') {
        Some(i) => (&path_query[..i], Some(&path_query[i + 1..])),
        None => (path_query, None),
    };
    let (path, query) = match path_q.find('?') {
        Some(i) => (&path_q[..i], Some(&path_q[i + 1..])),
        None => (path_q, None),
    };
    let query = query.map(sanitize_params).unwrap_or_default();
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    let fragment = match fragment {
        Some(f) if f.contains('=') => format!("#{}", sanitize_params(f)),
        Some(f) => format!("#{f}"),
        None => String::new(),
    };
    format!("{scheme}://{host}{path}{query}{fragment}{trailing}")
}

fn sanitize_params(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut out = Vec::new();
    let mut any = false;
    for pair in value.split('&') {
        let (k, v) = match pair.find('=') {
            Some(i) => (&pair[..i], Some(&pair[i + 1..])),
            None => (pair, None),
        };
        // Preserve pairs verbatim except redacted values (no re-encoding).
        match v {
            Some(_) if replacement_for_key(k) == Some(REDACTED_CREDENTIAL) => {
                out.push(format!("{k}={REDACTED_CREDENTIAL}"));
            }
            _ => out.push(pair.to_string()),
        }
        any = true;
    }
    if !any {
        return value.to_string();
    }
    out.join("&")
}

fn redact_ip_addresses(input: &str) -> String {
    // Candidates via regex, validity via std::net parsing (like ipaddress.*).
    let bracketed = re!(r"\[(?P<address>[0-9A-Fa-f:]+)\](?P<port>:\d{1,5})?");
    let mut safe = bracketed
        .replace_all(input, |caps: &regex::Captures| {
            match caps["address"].parse::<std::net::Ipv6Addr>() {
                Ok(_) => format!("[{}]{}", REDACTED_IP, &caps["port"]),
                Err(_) => caps[0].to_string(),
            }
        })
        .into_owned();
    // Bare IPv6: manual boundary handling (no look-around in Rust regex).
    let bare = re!(
        r"(^|[^0-9A-Fa-f:])(?P<address>[0-9A-Fa-f]{0,4}(?::[0-9A-Fa-f]{0,4}){2,7})([^0-9A-Fa-f:]|$)"
    );
    safe = bare
        .replace_all(&safe, |caps: &regex::Captures| {
            let addr = &caps[2];
            // Port suffix like :443 belongs to the match only if bracketed.
            match addr.parse::<std::net::Ipv6Addr>() {
                Ok(_) => format!("{}{}{}", &caps[1], REDACTED_IP, &caps[3]),
                Err(_) => caps[0].to_string(),
            }
        })
        .into_owned();
    let v4 = re!(r"(^|[^\w.])(?P<address>(?:\d{1,3}\.){3}\d{1,3})(?P<port>:\d{1,5})?([^\w.]|$)");
    v4.replace_all(&safe, |caps: &regex::Captures| {
        match caps["address"].parse::<std::net::Ipv4Addr>() {
            Ok(_) => format!("{}{}{}{}", &caps[1], REDACTED_IP, &caps["port"], &caps[4]),
            Err(_) => caps[0].to_string(),
        }
    })
    .into_owned()
}

fn redact_quoted_absolute_paths(input: &str) -> String {
    let r = re!(r#"(?P<open>["'])(?P<path>(?:[A-Za-z]:[\\/]|\\\\|/)[^"'\r\n]+)(?P<close>["'])"#);
    r.replace_all(input, |caps: &regex::Captures| {
        if caps["open"] != caps["close"] {
            return caps[0].to_string();
        }
        format!(
            "{}{}{}",
            &caps["open"],
            redacted_path_with_basename(&caps["path"]),
            &caps["open"]
        )
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn homes() -> Vec<String> {
        vec!["C:\\Users\\Alice".to_string(), "/home/alice".to_string()]
    }

    fn users() -> Vec<String> {
        vec!["Alice".to_string()]
    }

    fn s(input: &str) -> String {
        sanitize_with(input, &homes(), &users())
    }

    #[test]
    fn spec_f148_identity_fields() {
        let raw = "license_state: active\nlicense_id: CUSTOMER-123\nusername: Alice\nemail: alice@example.com";
        let safe = s(raw);
        assert!(safe.contains("license_state: active"), "{safe}");
        assert!(
            safe.contains(&format!("license_id: {REDACTED_LICENSE_FIELD}")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("username: {REDACTED_USER}")),
            "{safe}"
        );
        assert!(safe.contains(REDACTED_EMAIL), "{safe}");
        for f in ["CUSTOMER-123", "alice@example.com"] {
            assert!(!safe.contains(f), "{safe}");
        }
        assert!(
            s("Activation request code: MBD-ACTREQ-v1.abcXYZ0123456789-_")
                .contains(REDACTED_ACTIVATION_CODE)
        );
        assert!(s("machine_fingerprint_hash: abcdef0123456789").contains(REDACTED_FINGERPRINT));
    }

    #[test]
    fn spec_f148_paths_basename() {
        let safe = s("target: D:\\Games\\Private Pack\\mods");
        assert!(
            safe.contains(&format!("target: {REDACTED_PATH}\\mods")),
            "{safe}"
        );
        assert!(!safe.contains("Private Pack"), "{safe}");
        let safe = s("config_dir=/srv/private/profile/config");
        assert!(
            safe.contains(&format!("config_dir={REDACTED_PATH}/config")),
            "{safe}"
        );
    }

    #[test]
    fn spec_f148_idempotent_versions() {
        let raw = "license_state: missing\r\nminecraft_version: 1.21.1\r\nupdated: 2026-07-12T12:34:56Z\r\n";
        assert_eq!(s(raw), raw);
        assert_eq!(s(&s(raw)), s(raw));
    }

    #[test]
    fn spec_f148_headers_cli_privatekey() {
        let raw = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature\npassword=hunter2\napi-key: api-secret-value\ntoken_count: 4\npassword_check: missing\ncmd --token cli-secret -Dapi.key=jvm-secret";
        let safe = s(raw);
        assert!(
            safe.contains(&format!("Authorization: {REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("password={REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("api-key: {REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(safe.contains("token_count: 4"), "{safe}");
        assert!(safe.contains("password_check: missing"), "{safe}");
        assert!(
            safe.contains(&format!("--token {REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("-Dapi.key={REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        for f in [
            "hunter2",
            "api-secret-value",
            "cli-secret",
            "jvm-secret",
            "eyJhbGciOiJIUzI1NiJ9.payload.signature",
        ] {
            assert!(!safe.contains(f), "{safe}");
        }
        let pem = "-----BEGIN PRIVATE KEY-----\nprivate-material\n-----END PRIVATE KEY-----";
        assert!(!s(pem).contains("private-material"));
    }

    #[test]
    fn spec_f148_url_params() {
        let raw = "request=https://alice:pw@example.com/api/mods?token=query-secret&page=2&sort=updated#access_token=fragment-secret";
        let safe = s(raw);
        assert!(safe.contains("example.com/api/mods"), "{safe}");
        assert!(
            safe.contains(&format!("token={REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(safe.contains("page=2"), "{safe}");
        assert!(safe.contains("sort=updated"), "{safe}");
        assert!(
            safe.contains(&format!("access_token={REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        for f in ["query-secret", "fragment-secret"] {
            assert!(!safe.contains(f), "{safe}");
        }
        assert!(!safe.contains("alice:pw@"), "{safe}");
    }

    #[test]
    fn spec_f148_ip_versions() {
        let safe = s("server=192.168.1.50:25565 peer=[2001:db8::1]:443 minecraft=1.21.1 updated=2026-07-12T12:34:56Z");
        assert!(
            safe.contains(&format!("server={REDACTED_IP}:25565")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("peer=[{REDACTED_IP}]:443")),
            "{safe}"
        );
        assert!(safe.contains("minecraft=1.21.1"), "{safe}");
        assert!(safe.contains("2026-07-12T12:34:56Z"), "{safe}");
        assert!(!safe.contains("192.168.1.50"), "{safe}");
        assert!(!safe.contains("2001:db8::1"), "{safe}");
    }

    #[test]
    fn spec_f148_provider_keys() {
        let raw = "OPENAI_API_KEY=provider-secret MODRINTH_ACCESS_TOKEN=access-secret database_password=db-secret --curseforge-api-key cli-provider-secret url=https://example.com/api?cf_api_key=query-provider-secret&status=ok";
        let safe = s(raw);
        assert!(
            safe.contains(&format!("OPENAI_API_KEY={REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("--curseforge-api-key {REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(
            safe.contains(&format!("cf_api_key={REDACTED_CREDENTIAL}")),
            "{safe}"
        );
        assert!(safe.contains("status=ok"), "{safe}");
        for f in [
            "provider-secret",
            "access-secret",
            "db-secret",
            "cli-provider-secret",
            "query-provider-secret",
        ] {
            assert!(!safe.contains(f), "{safe}");
        }
    }

    #[test]
    fn spec_f148_json_recursive() {
        let raw = r#"{"status":"ok","token":"json-secret","nested":{"version":"1.21.1","url":"https://example.com/api?api_key=url-secret&limit=20"},"items":[{"email":"alice@example.com","count":3}]}"#;
        let safe = s(raw);
        let v: serde_json::Value = serde_json::from_str(&safe).expect("still json");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["token"], REDACTED_CREDENTIAL);
        assert_eq!(v["nested"]["version"], "1.21.1");
        assert!(v["nested"]["url"]
            .as_str()
            .expect("str")
            .contains(&format!("api_key={REDACTED_CREDENTIAL}")));
        assert!(v["nested"]["url"]
            .as_str()
            .expect("str")
            .contains("limit=20"));
        assert_eq!(v["items"][0]["email"], REDACTED_EMAIL);
        assert_eq!(v["items"][0]["count"], 3);
    }

    #[test]
    fn spec_f148_malformed_fallback_idempotent() {
        let raw =
            r#"{"token":"malformed-secret", "url":"https://example.com/?password=url-secret""#;
        let safe = s(raw);
        assert!(!safe.contains("malformed-secret"), "{safe}");
        assert!(!safe.contains("url-secret"), "{safe}");
        assert!(safe.contains(REDACTED_CREDENTIAL), "{safe}");
        assert_eq!(s(&safe), safe);
    }
}
