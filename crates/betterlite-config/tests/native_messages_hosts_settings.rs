#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::native_messages_hosts_settings::{HostEnvironment, Settings};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

#[derive(Clone, Copy)]
enum Host {
    Deepseek,
    Tencent,
    Minimax,
}

impl Host {
    fn of(self, settings: &Settings) -> &HostEnvironment {
        match self {
            Self::Deepseek => &settings.deepseek,
            Self::Tencent => &settings.tencent,
            Self::Minimax => &settings.minimax,
        }
    }
}

fn load(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .native_messages_hosts_settings
}

fn revealed(key: Option<&SecretString>) -> Option<String> {
    key.map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn native_messages_hosts_settings_default_to_nothing() {
    let settings = load("", &[]);
    assert!(settings.global_api_key.is_none());
    for host in [&settings.deepseek, &settings.tencent, &settings.minimax] {
        assert!(host.api_key.is_none());
        assert!(host.api_base.is_none());
        assert!(host.fallback_api_base.is_none());
    }
}

#[rstest]
#[case::deepseek(Host::Deepseek, &[("DEEPSEEK_API_KEY", "deepseek-key"), ("DEEPSEEK_ANTHROPIC_API_BASE", "https://anthropic.deepseek.test"), ("DEEPSEEK_API_BASE", "https://deepseek.test")], "deepseek-key", "https://anthropic.deepseek.test", Some("https://deepseek.test"))]
#[case::tencent(Host::Tencent, &[("TENCENT_API_KEY", "tencent-key"), ("TENCENT_ANTHROPIC_API_BASE", "https://anthropic.tencent.test"), ("TENCENT_API_BASE", "https://tencent.test")], "tencent-key", "https://anthropic.tencent.test", Some("https://tencent.test"))]
#[case::minimax(Host::Minimax, &[("MINIMAX_API_KEY", "minimax-key"), ("MINIMAX_API_BASE", "https://minimax.test")], "minimax-key", "https://minimax.test", None)]
fn native_messages_hosts_settings_read_each_alias(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: &str,
    #[case] expected_base: &str,
    #[case] expected_fallback: Option<&str>,
) {
    let settings = load("", environment);
    let loaded = host.of(&settings);
    assert_eq!(
        revealed(loaded.api_key.as_ref()).as_deref(),
        Some(expected_key)
    );
    assert_eq!(loaded.api_base.as_deref(), Some(expected_base));
    assert_eq!(loaded.fallback_api_base.as_deref(), expected_fallback);
}

#[rstest]
#[case::deepseek(Host::Deepseek, &[("DEEPSEEK_API_KEY", ""), ("DEEPSEEK_ANTHROPIC_API_BASE", ""), ("DEEPSEEK_API_BASE", "")])]
#[case::tencent(Host::Tencent, &[("TENCENT_API_KEY", ""), ("TENCENT_ANTHROPIC_API_BASE", ""), ("TENCENT_API_BASE", "")])]
#[case::minimax(Host::Minimax, &[("MINIMAX_API_KEY", ""), ("MINIMAX_API_BASE", "")])]
fn native_messages_hosts_settings_ignore_empty_aliases(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
) {
    let settings = load("", environment);
    let loaded = host.of(&settings);
    assert!(loaded.api_key.is_none());
    assert!(loaded.api_base.is_none());
    assert!(loaded.fallback_api_base.is_none());
}

#[rstest]
#[case::module_key("litellm_settings:\n  api_key: global-key\n", &[], Some("global-key"))]
#[case::environment_reference("litellm_settings:\n  api_key: os.environ/SHARED_KEY\n", &[("SHARED_KEY", "referenced-key")], Some("referenced-key"))]
#[case::absent("", &[], None)]
fn native_messages_hosts_settings_load_the_global_key(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load(source, environment);
    assert_eq!(
        revealed(settings.global_api_key.as_ref()).as_deref(),
        expected
    );
}
