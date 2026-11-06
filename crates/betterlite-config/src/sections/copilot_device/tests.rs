#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rstest::rstest;

use crate::loader::{ConfigLoader, EnvironmentLookup};
use crate::sections::copilot_device::Settings;
use crate::types::Config;

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(pairs: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: lookup(pairs),
    }
    .load("")
    .expect("a document with no copilot_device keys still loads")
    .sections
    .copilot_device
}

#[rstest]
#[case::no_home(&[], "~/.config/litellm/github_copilot")]
#[case::home(&[("HOME", "/home/alice")], "/home/alice/.config/litellm/github_copilot")]
#[case::empty_home(&[("HOME", "")], "/.config/litellm/github_copilot")]
#[case::trailing_home(&[("HOME", "/home/alice/")], "/home/alice/.config/litellm/github_copilot")]
fn copilot_device_defaults(#[case] environment: &[(&str, &str)], #[case] token_dir: &str) {
    let settings = settings(environment);

    assert_eq!(settings.token_dir, token_dir);
    assert_eq!(settings.access_token_file, "access-token");
    assert_eq!(settings.api_key_file, "api-key.json");
    assert_eq!(settings.client_id, "Iv1.b507a08c87ecfe98");
    assert_eq!(
        settings.device_code_url,
        "https://github.com/login/device/code"
    );
    assert_eq!(
        settings.access_token_url,
        "https://github.com/login/oauth/access_token"
    );
    assert_eq!(
        settings.api_key_url,
        "https://api.github.com/copilot_internal/v2/token"
    );
}

#[test]
fn copilot_device_overrides() {
    let settings = settings(&[
        ("GITHUB_COPILOT_TOKEN_DIR", ""),
        ("GITHUB_COPILOT_ACCESS_TOKEN_FILE", "token.txt"),
        ("GITHUB_COPILOT_API_KEY_FILE", "key.json"),
        ("GITHUB_COPILOT_CLIENT_ID", "Iv1.custom"),
        (
            "GITHUB_COPILOT_DEVICE_CODE_URL",
            "https://device.example/code",
        ),
        (
            "GITHUB_COPILOT_ACCESS_TOKEN_URL",
            "https://token.example/token",
        ),
        ("GITHUB_COPILOT_API_KEY_URL", "https://key.example/key"),
    ]);

    assert_eq!(settings.token_dir, "");
    assert_eq!(settings.access_token_file, "token.txt");
    assert_eq!(settings.api_key_file, "key.json");
    assert_eq!(settings.client_id, "Iv1.custom");
    assert_eq!(settings.device_code_url, "https://device.example/code");
    assert_eq!(settings.access_token_url, "https://token.example/token");
    assert_eq!(settings.api_key_url, "https://key.example/key");
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

fn home_config(environment: &[(&str, &str)], home: Option<&str>) -> Config {
    ConfigLoader {
        lookup: HomeEnvironment {
            variables: lookup(environment),
            home: home.map(PathBuf::from),
        },
    }
    .load("")
    .expect("injected home settings load")
}

#[rstest]
#[case::system_home(&[], Some("/system/user"), "/system/user/.config/litellm/github_copilot")]
#[case::no_system_home(&[], None, "~/.config/litellm/github_copilot")]
#[case::home_wins(&[("HOME", "/env/user")], Some("/system/user"), "/env/user/.config/litellm/github_copilot")]
#[case::empty_home(&[("HOME", "")], Some("/system/user"), "/.config/litellm/github_copilot")]
#[case::empty_system_home(&[], Some(""), "/.config/litellm/github_copilot")]
#[case::trailing_system_home(&[], Some("/system/user///"), "/system/user/.config/litellm/github_copilot")]
fn copilot_device_home_sources(
    #[case] environment: &[(&str, &str)],
    #[case] home: Option<&str>,
    #[case] token_dir: &str,
) {
    let config = home_config(environment, home);

    assert_eq!(config.sections.copilot_device.token_dir, token_dir);
}

#[cfg(unix)]
#[test]
fn copilot_device_rejects_a_non_utf8_system_home() {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::OsStringExt as _;

    use tracing::subscriber::with_default;

    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let loader = ConfigLoader {
        lookup: HomeEnvironment {
            variables: BTreeMap::new(),
            home: Some(PathBuf::from(OsString::from_vec(vec![b'/', 0xff]))),
        },
    };
    let config = with_default(subscriber, || loader.load(""))
        .expect("an unrepresentable home keeps the literal default");
    assert_eq!(
        config.sections.copilot_device.token_dir,
        "~/.config/litellm/github_copilot"
    );
    assert_eq!(
        fs::read_to_string(log.path()).unwrap(),
        " WARN the system home is not UTF-8; the Copilot token path keeps its literal default\n"
    );
}
