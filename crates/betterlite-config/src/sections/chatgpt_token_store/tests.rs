#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rstest::rstest;

use crate::loader::{ConfigLoader, EnvironmentLookup};

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::defaults(
    &[],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::home_expands_default(
    &[("HOME", "/home/alice")],
    "/home/alice/.config/litellm/chatgpt",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::empty_home(&[("HOME", "")], "/.config/litellm/chatgpt", "auth.json", "https://chatgpt.com/backend-api/codex")]
#[case::trailing_home(&[("HOME", "/home/alice/")], "/home/alice/.config/litellm/chatgpt", "auth.json", "https://chatgpt.com/backend-api/codex")]
#[case::literal_override(&[("CHATGPT_TOKEN_DIR", "~/literal")], "~/literal", "auth.json", "https://chatgpt.com/backend-api/codex")]
#[case::primary_wins(&[("CHATGPT_API_BASE", "primary"), ("OPENAI_CHATGPT_API_BASE", "secondary")], "~/.config/litellm/chatgpt", "auth.json", "primary")]
#[case::empty_bases(&[("CHATGPT_API_BASE", ""), ("OPENAI_CHATGPT_API_BASE", "")], "~/.config/litellm/chatgpt", "auth.json", "https://chatgpt.com/backend-api/codex")]
#[case::explicit_dir(
    &[("CHATGPT_TOKEN_DIR", "/custom")],
    "/custom",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_empty_dir(
    &[("CHATGPT_TOKEN_DIR", "")],
    "",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::absolute_file(
    &[("CHATGPT_AUTH_FILE", "/absolute/auth.json")],
    "~/.config/litellm/chatgpt",
    "/absolute/auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_file(
    &[("CHATGPT_AUTH_FILE", "other.json")],
    "~/.config/litellm/chatgpt",
    "other.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_empty_file(
    &[("CHATGPT_AUTH_FILE", "")],
    "~/.config/litellm/chatgpt",
    "",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::primary_api_base(
    &[("CHATGPT_API_BASE", "https://primary.example")],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://primary.example"
)]
#[case::secondary_api_base(
    &[("OPENAI_CHATGPT_API_BASE", "https://secondary.example")],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://secondary.example"
)]
#[case::empty_primary_falls_through(
    &[
        ("CHATGPT_API_BASE", ""),
        ("OPENAI_CHATGPT_API_BASE", "https://secondary.example"),
    ],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://secondary.example"
)]
fn chatgpt_token_paths_load(
    #[case] env: &[(&str, &str)],
    #[case] token_dir: &str,
    #[case] auth_file: &str,
    #[case] api_base: &str,
) {
    let settings = ConfigLoader {
        lookup: lookup(env),
    }
    .load("")
    .expect("a document with no chatgpt_token_store keys still loads")
    .sections
    .chatgpt_token_store;

    assert_eq!(settings.token_dir, token_dir);
    assert_eq!(settings.auth_file, auth_file);
    assert_eq!(settings.api_base, api_base);
}

struct HomeEnvironment {
    variables: BTreeMap<String, String>,
    home: Option<PathBuf>,
}

impl EnvironmentLookup for HomeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[rstest]
#[case::absent_home(&[], Some("/system/user"), "/system/user/.config/litellm/chatgpt")]
#[case::no_system_home(&[], None, "~/.config/litellm/chatgpt")]
#[case::present_home(&[("HOME", "/env/user")], Some("/system/user"), "/env/user/.config/litellm/chatgpt")]
#[case::empty_home(&[("HOME", "")], Some("/system/user"), "/.config/litellm/chatgpt")]
#[case::empty_system_home(&[], Some(""), "/.config/litellm/chatgpt")]
#[case::trailing_system_home(&[], Some("/system/user///"), "/system/user/.config/litellm/chatgpt")]
#[case::empty_override(&[("CHATGPT_TOKEN_DIR", "")], Some("/system/user"), "")]
#[case::literal_override(&[("CHATGPT_TOKEN_DIR", "~/literal")], Some("/system/user"), "~/literal")]
fn chatgpt_token_paths_load_home(
    #[case] env: &[(&str, &str)],
    #[case] home: Option<&str>,
    #[case] expected: &str,
) {
    let config = ConfigLoader {
        lookup: HomeEnvironment {
            variables: lookup(env),
            home: home.map(PathBuf::from),
        },
    }
    .load("")
    .expect("injected home settings load");
    assert_eq!(config.sections.chatgpt_token_store.token_dir, expected);
}

#[cfg(unix)]
#[test]
fn chatgpt_token_paths_load_non_utf8_home() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let config = ConfigLoader {
        lookup: HomeEnvironment {
            variables: BTreeMap::new(),
            home: Some(PathBuf::from(OsString::from_vec(vec![b'/', 0xff]))),
        },
    }
    .load("")
    .expect("an unrepresentable home keeps the literal default");
    assert_eq!(
        config.sections.chatgpt_token_store.token_dir,
        "~/.config/litellm/chatgpt"
    );
}
