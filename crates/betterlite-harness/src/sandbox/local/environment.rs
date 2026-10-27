use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

const SECRET_PREFIXES: [&str; 12] = [
    "ANTHROPIC_",
    "OPENAI_",
    "LITELLM_",
    "AZURE_",
    "AWS_",
    "GEMINI_",
    "CODEX_",
    "CURSOR_",
    "VERTEX",
    "CLAUDE_CODE_",
    "CLAUDE_PID",
    "CLAUDECODE",
];

#[must_use]
pub fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper == "GOOGLE_APPLICATION_CREDENTIALS"
        || SECRET_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
        || ["API_KEY", "TOKEN", "SECRET"]
            .iter()
            .any(|part| upper.contains(part))
}

#[must_use]
pub fn filtered_environ(
    base: &BTreeMap<String, SecretString>,
    extra: &BTreeMap<String, SecretString>,
) -> BTreeMap<String, SecretString> {
    let mut kept: BTreeMap<_, _> = base
        .iter()
        .filter(|(name, _value)| !is_secret_env_name(name))
        .map(|(name, value)| (name.clone(), SecretString::from(value.expose_secret())))
        .collect();
    kept.extend(
        extra
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.expose_secret()))),
    );
    kept
}
