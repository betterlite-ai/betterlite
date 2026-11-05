#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::chatgpt_token_store::home::HomeEnvironment;

const TOKEN_DIR: &str = "~/.config/litellm/xai_oauth";
const API_BASE: &str = "https://api.x.ai/v1";

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::unset_defaults(&[], (TOKEN_DIR, "auth.json", API_BASE))]
#[case::home_expands_default(&[("HOME", "/home/alice")], ("/home/alice/.config/litellm/xai_oauth", "auth.json", API_BASE))]
#[case::explicit_token_dir(&[("XAI_OAUTH_TOKEN_DIR", "/custom")], ("/custom", "auth.json", API_BASE))]
#[case::literal_tilde_dir(&[("XAI_OAUTH_TOKEN_DIR", "~/literal")], ("~/literal", "auth.json", API_BASE))]
#[case::empty_token_dir_is_unset(&[("XAI_OAUTH_TOKEN_DIR", "")], (TOKEN_DIR, "auth.json", API_BASE))]
#[case::explicit_auth_file(&[("XAI_OAUTH_AUTH_FILE", "other.json")], (TOKEN_DIR, "other.json", API_BASE))]
#[case::empty_auth_file_is_unset(&[("XAI_OAUTH_AUTH_FILE", "")], (TOKEN_DIR, "auth.json", API_BASE))]
#[case::oauth_base(&[("XAI_OAUTH_API_BASE", "https://oauth.example/v1")], (TOKEN_DIR, "auth.json", "https://oauth.example/v1"))]
#[case::shared_base(&[("XAI_API_BASE", "https://shared.example/v1")], (TOKEN_DIR, "auth.json", "https://shared.example/v1"))]
#[case::oauth_base_wins(
    &[("XAI_OAUTH_API_BASE", "https://oauth.example/v1"), ("XAI_API_BASE", "https://shared.example/v1")],
    (TOKEN_DIR, "auth.json", "https://oauth.example/v1")
)]
#[case::empty_oauth_base_falls_through(
    &[("XAI_OAUTH_API_BASE", ""), ("XAI_API_BASE", "https://shared.example/v1")],
    (TOKEN_DIR, "auth.json", "https://shared.example/v1")
)]
#[case::empty_bases_use_default(&[("XAI_OAUTH_API_BASE", ""), ("XAI_API_BASE", "")], (TOKEN_DIR, "auth.json", API_BASE))]
fn xai_oauth_settings_resolve_environment(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (&str, &str, &str),
) {
    let settings = ConfigLoader {
        lookup: lookup(environment),
    }
    .load("")
    .expect("a document with no xai keys loads")
    .sections
    .xai_oauth_settings;

    assert_eq!(settings.token_dir, expected.0);
    assert_eq!(settings.auth_file, expected.1);
    assert_eq!(settings.api_base, expected.2);
}

#[rstest]
#[case::absent_home(&[], Some("/system/user"), "/system/user/.config/litellm/xai_oauth")]
#[case::no_system_home(&[], None, TOKEN_DIR)]
#[case::environment_home_wins(&[("HOME", "/env/user")], Some("/system/user"), "/env/user/.config/litellm/xai_oauth")]
#[case::empty_home(&[("HOME", "")], Some("/system/user"), "/.config/litellm/xai_oauth")]
#[case::trailing_slashes(&[("HOME", "/env/user//")], None, "/env/user/.config/litellm/xai_oauth")]
#[case::empty_override_uses_home(&[("XAI_OAUTH_TOKEN_DIR", "")], Some("/system/user"), "/system/user/.config/litellm/xai_oauth")]
fn xai_oauth_settings_expand_home(
    #[case] environment: &[(&str, &str)],
    #[case] home: Option<&str>,
    #[case] expected: &str,
) {
    let config = ConfigLoader {
        lookup: HomeEnvironment {
            variables: lookup(environment),
            home: home.map(PathBuf::from),
        },
    }
    .load("")
    .expect("injected home settings load");

    assert_eq!(config.sections.xai_oauth_settings.token_dir, expected);
}

fn deployments(params: &[&str]) -> String {
    let mut source = "model_list:\n".to_owned();
    for (index, param) in params.iter().enumerate() {
        write!(
            source,
            "  - model_name: m{index}\n    litellm_params:\n      model: xai/grok\n{param}"
        )
        .expect("writing to a String cannot fail");
    }
    source
}

#[rstest]
#[case::absent("", Ok(false))]
#[case::true_flag("      use_xai_oauth: true\n", Ok(true))]
#[case::false_flag("      use_xai_oauth: false\n", Ok(false))]
#[case::null_flag("      use_xai_oauth: null\n", Ok(false))]
#[case::nonzero_integer("      use_xai_oauth: 2\n", Ok(true))]
#[case::zero_integer("      use_xai_oauth: 0\n", Ok(false))]
#[case::nonzero_float("      use_xai_oauth: 0.5\n", Ok(true))]
#[case::zero_float("      use_xai_oauth: 0.0\n", Ok(false))]
#[case::non_empty_text("      use_xai_oauth: 'false'\n", Ok(true))]
#[case::empty_text("      use_xai_oauth: ''\n", Ok(false))]
#[case::non_empty_list("      use_xai_oauth: [0]\n", Ok(true))]
#[case::empty_list("      use_xai_oauth: []\n", Ok(false))]
#[case::non_empty_mapping("      use_xai_oauth: {a: 0}\n", Ok(true))]
#[case::empty_mapping("      use_xai_oauth: {}\n", Ok(false))]
#[case::environment_true("      use_xai_oauth: os.environ/XAI_FLAG\n", Ok(true))]
#[case::environment_false("      use_xai_oauth: os.environ/FALSE_FLAG\n", Ok(false))]
#[case::environment_off("      use_xai_oauth: os.environ/OFF_FLAG\n", Ok(false))]
#[case::environment_empty("      use_xai_oauth: os.environ/EMPTY_FLAG\n", Ok(false))]
fn xai_oauth_settings_read_the_deployment_flag(
    #[case] param: &str,
    #[case] expected: Result<bool, ()>,
) {
    let config = ConfigLoader {
        lookup: lookup(&[
            ("XAI_FLAG", "yes"),
            ("FALSE_FLAG", "false"),
            ("OFF_FLAG", "off"),
            ("EMPTY_FLAG", ""),
        ]),
    }
    .load(&deployments(&[param]))
    .expect("the flag loads");

    assert_eq!(
        Ok(config.sections.xai_oauth_settings.deployments[0]),
        expected
    );
}

#[test]
fn xai_oauth_settings_reject_an_unreadable_flag_variable() {
    let error = ConfigLoader {
        lookup: lookup(&[("XAI_FLAG", "sometimes")]),
    }
    .load(&deployments(&[
        "      use_xai_oauth: os.environ/XAI_FLAG\n",
    ]))
    .expect_err("a value that is not a flag is rejected");

    let ConfigError::Environment { source } = &error else {
        panic!("{error:?} is not an environment error");
    };
    assert!(source.to_string().contains("XAI_FLAG"));
}

#[test]
fn xai_oauth_settings_name_a_missing_flag_variable() {
    let error = ConfigLoader {
        lookup: lookup(&[]),
    }
    .load(&deployments(&[
        "      use_xai_oauth: os.environ/XAI_FLAG\n",
    ]))
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "XAI_FLAG" && path == "model_list.0.litellm_params.use_xai_oauth"
    ));
}

#[test]
fn xai_oauth_settings_keep_one_flag_per_deployment_in_file_order() {
    let config = ConfigLoader {
        lookup: lookup(&[]),
    }
    .load(&deployments(&[
        "      use_xai_oauth: true\n",
        "",
        "      use_xai_oauth: 1\n",
    ]))
    .expect("three deployments load");

    assert_eq!(
        config.sections.xai_oauth_settings.deployments,
        [true, false, true]
    );
}
