//! Secret paths and credential redaction.

use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

/// Directory names whose contents are always treated as secrets.
const SECRET_DIRS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".password-store",
    ".azure",
    ".kube",
    ".docker",
    "Keychains",
    ".1password",
];

/// Exact file names treated as secrets.
const SECRET_FILES: &[&str] = &[
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".netrc",
    "_netrc",
    ".pgpass",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    "credentials",
    "credentials.json",
    "service-account.json",
    "Login Data",
    "logins.json",
    "key4.db",
    "Cookies",
    "master.key",
    ".htpasswd",
    "secrets.yml",
    "secrets.yaml",
    "terraform.tfstate",
];

const SECRET_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "keystore", "jks", "kdbx"];

/// Whether a path likely holds credentials. `.env.example`, `.env.sample`
/// and `.env.template` are documentation, not secrets.
pub fn is_secret_path(p: &Path) -> bool {
    for comp in p.components() {
        let s = comp.as_os_str().to_string_lossy();
        if SECRET_DIRS.iter().any(|d| s.eq_ignore_ascii_case(d)) {
            return true;
        }
        // gcloud keeps credentials under ~/.config/gcloud
        if s == "gcloud" {
            return true;
        }
    }
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return false;
    };
    if SECRET_FILES.iter().any(|f| name == *f) {
        return true;
    }
    if name.starts_with("id_") && !name.ends_with(".pub") && p.to_string_lossy().contains(".ssh") {
        return true;
    }
    if name == ".env" || (name.starts_with(".env.") && !is_env_template(&name)) {
        return true;
    }
    if let Some(ext) = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) {
        if SECRET_EXTENSIONS.contains(&ext.as_str()) {
            return true;
        }
    }
    false
}

fn is_env_template(name: &str) -> bool {
    [".example", ".sample", ".template", ".dist", ".defaults"]
        .iter()
        .any(|s| name.ends_with(s))
}

/// Path fragments that indicate a secret when they appear inside a shell
/// command string.
pub const SECRET_COMMAND_FRAGMENTS: &[&str] = &[
    ".ssh/",
    "/.ssh",
    "~/.ssh",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    ".aws/",
    "~/.aws",
    ".gnupg",
    ".netrc",
    ".git-credentials",
    ".password-store",
    "Keychains",
    "Login Data",
    "logins.json",
    ".kube/config",
    ".docker/config.json",
    "/etc/shadow",
    "gcloud/",
];

struct Rule {
    name: &'static str,
    re: Regex,
    /// Capture group holding the secret; 0 = whole match.
    group: usize,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |name, pat: &str, group| Rule {
            name,
            re: Regex::new(pat).expect("valid secret regex"),
            group,
        };
        vec![
            r(
                "private-key",
                r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)",
                0,
            ),
            r("aws-access-key-id", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b", 0),
            r(
                "aws-secret",
                r#"(?i)aws_secret_access_key\s*[:=]\s*["']?([A-Za-z0-9/+=]{40})"#,
                1,
            ),
            r("github-token", r"\b(gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b", 0),
            r("slack-token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b", 0),
            r("stripe-key", r"\b[sr]k_live_[A-Za-z0-9]{16,}\b", 0),
            r("google-api-key", r"\bAIza[0-9A-Za-z_\-]{35}\b", 0),
            r("api-key", r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_\-]{24,}\b", 0),
            r(
                "jwt",
                r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
                0,
            ),
            r(
                "assigned-secret",
                r#"(?i)\b[\w.-]*(?:password|passwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|private[_-]?key)[\w.-]*\s*[:=]\s*["']([^"'\s]{8,})["']"#,
                1,
            ),
            r(
                "url-credentials",
                r"\b[a-z][a-z0-9+.-]*://[^/\s:@]+:([^/\s:@]{4,})@",
                1,
            ),
        ]
    })
}

/// Replace credentials in `text` with `[REDACTED:<kind>]`. Returns the
/// redacted text and the number of redactions.
pub fn redact(text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0;
    for rule in rules() {
        let mut result = String::with_capacity(out.len());
        let mut last = 0;
        for caps in rule.re.captures_iter(&out) {
            let Some(m) = caps.get(rule.group) else {
                continue;
            };
            if m.as_str().contains("[REDACTED") {
                continue;
            }
            result.push_str(&out[last..m.start()]);
            result.push_str(&format!("[REDACTED:{}]", rule.name));
            last = m.end();
            count += 1;
        }
        result.push_str(&out[last..]);
        out = result;
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn secret_paths() {
        for p in [
            "/home/u/.ssh/id_rsa",
            "/home/u/.ssh/config",
            "/home/u/.aws/credentials",
            "/proj/.env",
            "/proj/.env.production",
            "/proj/certs/server.key",
            "/proj/config/master.key",
            "/home/u/.config/gcloud/application_default_credentials.json",
            "/Users/u/Library/Keychains/login.keychain-db",
        ] {
            assert!(is_secret_path(&PathBuf::from(p)), "{p}");
        }
        for p in [
            "/proj/.env.example",
            "/proj/src/env.rs",
            "/proj/README.md",
            "/proj/src/keys.rs",
            "/proj/app/models/secret_santa.rb",
        ] {
            assert!(!is_secret_path(&PathBuf::from(p)), "{p}");
        }
    }

    #[test]
    fn redacts_common_credentials() {
        let input = "\
AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
token: ghp_abcdefghijklmnopqrstuvwxyz0123456789
DATABASE_PASSWORD = \"hunter2hunter2\"
db = postgres://admin:sup3rs3cret@db.internal:5432/app
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAA
-----END OPENSSH PRIVATE KEY-----
";
        let (out, n) = redact(input);
        assert!(n >= 6, "{n}: {out}");
        for leaked in [
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI",
            "ghp_abcdef",
            "hunter2hunter2",
            "sup3rs3cret",
            "b3BlbnNzaC1rZXktdjEAAAAA",
        ] {
            assert!(!out.contains(leaked), "leaked {leaked}: {out}");
        }
        assert!(out.contains("postgres://admin:[REDACTED:url-credentials]@db.internal"));
    }

    #[test]
    fn leaves_ordinary_code_alone() {
        let code = "def authenticate(password)\n  user.password == params[:password]\nend\n";
        let (out, n) = redact(code);
        assert_eq!(n, 0);
        assert_eq!(out, code);
    }
}
