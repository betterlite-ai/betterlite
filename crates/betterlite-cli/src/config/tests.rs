#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::error::PrivateJsonError;
use rstest::rstest;
use tempfile::TempDir;

use super::commands::{ConfigCommand, run_config_command};
use super::error::ConfigError;
use super::{
    HIDDEN_COMMANDS_KEY, config_file_path, hidden_command_names, load_config, normalize_base_url,
    normalize_hidden_commands, parse_hidden_commands, save_config,
};

const FULL_URL_MESSAGE: &str = "base_url must be a full http:// or https:// URL including a host";
const CONTROL_CHARACTER_MESSAGE: &str =
    "base_url must not contain whitespace or control characters";
const QUERY_MESSAGE: &str = "base_url must not include a query string or fragment";
const CREDENTIALS_MESSAGE: &str =
    "base_url must not contain credentials; pass --api-key separately";
const EMPTY_LIST_MESSAGE: &str = "hidden_commands must be a comma-separated list of command names, e.g. `lite config set hidden_commands codex,opencode`. To list everything again, run `lite config unset hidden_commands`";

struct FakeEnvironment {
    variables: BTreeMap<String, String>,
    home: Option<PathBuf>,
}

impl FakeEnvironment {
    fn with_home(home: &Path) -> Self {
        Self {
            variables: BTreeMap::new(),
            home: Some(home.to_owned()),
        }
    }

    fn without_home() -> Self {
        Self {
            variables: BTreeMap::new(),
            home: None,
        }
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

struct Output {
    result: Result<(), ConfigError>,
    stdout: String,
    stderr: String,
}

fn run(command: ConfigCommand, environment: &FakeEnvironment) -> Output {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = run_config_command(command, environment, &mut stdout, &mut stderr);
    Output {
        result,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn set(key: &str, value: &str) -> ConfigCommand {
    ConfigCommand::Set {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn config_path(home: &Path) -> PathBuf {
    home.join(".litellm").join("config.json")
}

fn write_config(home: &Path, contents: &str) {
    fs::create_dir_all(home.join(".litellm")).unwrap();
    fs::write(config_path(home), contents).unwrap();
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn config_file_path_joins_the_home_directory() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    assert_eq!(
        config_file_path(&environment),
        Some(home.path().join(".litellm").join("config.json"))
    );
}

#[test]
fn config_file_path_returns_none_without_a_home() {
    let environment = FakeEnvironment::without_home();
    assert_eq!(config_file_path(&environment), None);
}

#[test]
fn config_file_path_creates_no_directory() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    assert!(config_file_path(&environment).is_some());
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn load_config_returns_an_empty_map_for_a_missing_file() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    assert!(load_config(&environment, &mut warnings).is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn load_config_returns_an_empty_map_without_a_home() {
    let environment = FakeEnvironment::without_home();
    let mut warnings = Vec::new();
    assert!(load_config(&environment, &mut warnings).is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn load_config_returns_the_stored_entries() {
    let home = TempDir::new().unwrap();
    write_config(
        home.path(),
        r#"{"base_url": "https://proxy.example.com", "hidden_commands": "codex"}"#,
    );
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    let config = load_config(&environment, &mut warnings);
    let expected = BTreeMap::from([
        (
            "base_url".to_owned(),
            "https://proxy.example.com".to_owned(),
        ),
        ("hidden_commands".to_owned(), "codex".to_owned()),
    ]);
    assert_eq!(config, expected);
    assert!(warnings.is_empty());
}

#[test]
fn load_config_warns_about_invalid_json() {
    let home = TempDir::new().unwrap();
    write_config(home.path(), "not json");
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    assert!(load_config(&environment, &mut warnings).is_empty());
    let failure = serde_json::from_str::<BTreeMap<String, String>>("not json").unwrap_err();
    assert_eq!(
        String::from_utf8(warnings).unwrap(),
        format!(
            "Warning: ignoring invalid config file {}: {failure}\n",
            config_path(home.path()).display()
        )
    );
}

#[test]
fn load_config_warns_about_a_value_that_is_not_a_string() {
    let home = TempDir::new().unwrap();
    let stored = r#"{"base_url": 8}"#;
    write_config(home.path(), stored);
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    assert!(load_config(&environment, &mut warnings).is_empty());
    let failure = serde_json::from_str::<BTreeMap<String, String>>(stored).unwrap_err();
    assert!(failure.to_string().contains("expected a string"));
    assert_eq!(
        String::from_utf8(warnings).unwrap(),
        format!(
            "Warning: ignoring invalid config file {}: {failure}\n",
            config_path(home.path()).display()
        )
    );
}

#[test]
fn load_config_warns_when_the_file_cannot_be_read() {
    let home = TempDir::new().unwrap();
    let path = config_path(home.path());
    fs::create_dir_all(&path).unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    assert!(load_config(&environment, &mut warnings).is_empty());
    let failure = fs::read_to_string(&path).unwrap_err();
    assert_eq!(
        String::from_utf8(warnings).unwrap(),
        format!(
            "Warning: ignoring invalid config file {}: {failure}\n",
            path.display()
        )
    );
}

#[test]
fn save_config_writes_the_map_below_the_home() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let config = BTreeMap::from([(
        "base_url".to_owned(),
        "https://proxy.example.com".to_owned(),
    )]);
    let path = save_config(&config, &environment).unwrap();
    assert_eq!(path, config_path(home.path()));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"base_url\": \"https://proxy.example.com\"\n}"
    );
}

#[test]
fn save_config_sets_owner_only_modes() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let path = save_config(&BTreeMap::new(), &environment).unwrap();
    assert_eq!(mode(&home.path().join(".litellm")), 0o700);
    assert_eq!(mode(&path), 0o600);
}

#[test]
fn save_config_tightens_a_group_readable_directory() {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".litellm");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    save_config(&BTreeMap::new(), &environment).unwrap();
    assert_eq!(mode(&directory), 0o700);
}

#[test]
fn save_config_round_trips_through_load_config() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let config = BTreeMap::from([
        (
            "base_url".to_owned(),
            "https://proxy.example.com".to_owned(),
        ),
        ("hidden_commands".to_owned(), "codex,opencode".to_owned()),
    ]);
    save_config(&config, &environment).unwrap();
    let mut warnings = Vec::new();
    assert_eq!(load_config(&environment, &mut warnings), config);
    assert!(warnings.is_empty());
}

#[test]
fn save_config_reports_a_missing_home() {
    let environment = FakeEnvironment::without_home();
    let error = save_config(&BTreeMap::new(), &environment).unwrap_err();
    assert!(matches!(error, ConfigError::NoHome));
}

#[test]
fn save_config_wraps_a_directory_failure() {
    let home = TempDir::new().unwrap();
    let blocked = home.path().join("blocked");
    fs::write(&blocked, "occupied").unwrap();
    let environment = FakeEnvironment::with_home(&blocked);
    let error = save_config(&BTreeMap::new(), &environment).unwrap_err();
    match error {
        ConfigError::Save(PrivateJsonError::Directory(path, _)) => {
            assert_eq!(path, blocked.join(".litellm"));
        }
        ConfigError::Usage(message) => panic!("unexpected usage failure: {message}"),
        ConfigError::KeyNotSet(key) => panic!("unexpected key failure: {key}"),
        ConfigError::NoHome => panic!("unexpected missing-home failure"),
        ConfigError::Runtime { source } => panic!("unexpected runtime failure: {source}"),
        ConfigError::Save(other) => panic!("unexpected save failure: {other:?}"),
    }
}

#[test]
fn save_config_wraps_a_write_failure() {
    let home = TempDir::new().unwrap();
    let target = config_path(home.path());
    fs::create_dir_all(&target).unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let error = save_config(&BTreeMap::new(), &environment).unwrap_err();
    match error {
        ConfigError::Save(PrivateJsonError::Commit(path, _)) => {
            assert_eq!(path, target);
        }
        ConfigError::Usage(message) => panic!("unexpected usage failure: {message}"),
        ConfigError::KeyNotSet(key) => panic!("unexpected key failure: {key}"),
        ConfigError::NoHome => panic!("unexpected missing-home failure"),
        ConfigError::Runtime { source } => panic!("unexpected runtime failure: {source}"),
        ConfigError::Save(other) => panic!("unexpected save failure: {other:?}"),
    }
}

#[test]
fn set_config_saves_the_normalized_value_and_prints_it() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(
        set("base_url", "https://proxy.example.com///"),
        &environment,
    );
    output.result.unwrap();
    assert_eq!(
        output.stdout,
        format!(
            "Set base_url = https://proxy.example.com in {}\n",
            config_path(home.path()).display()
        )
    );
    assert_eq!(output.stderr, "");
    assert_eq!(
        fs::read_to_string(config_path(home.path())).unwrap(),
        "{\n  \"base_url\": \"https://proxy.example.com\"\n}"
    );
}

#[test]
fn set_config_normalizes_hidden_commands() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(
        set(HIDDEN_COMMANDS_KEY, " opencode, codex ,,"),
        &environment,
    );
    output.result.unwrap();
    assert_eq!(
        output.stdout,
        format!(
            "Set hidden_commands = codex,opencode in {}\n",
            config_path(home.path()).display()
        )
    );
    assert_eq!(
        fs::read_to_string(config_path(home.path())).unwrap(),
        "{\n  \"hidden_commands\": \"codex,opencode\"\n}"
    );
}

#[test]
fn set_config_merges_with_the_stored_config() {
    let home = TempDir::new().unwrap();
    write_config(home.path(), r#"{"base_url": "https://stored.example.com"}"#);
    let environment = FakeEnvironment::with_home(home.path());
    run(set(HIDDEN_COMMANDS_KEY, "codex"), &environment)
        .result
        .unwrap();
    assert_eq!(
        fs::read_to_string(config_path(home.path())).unwrap(),
        "{\n  \"base_url\": \"https://stored.example.com\",\n  \"hidden_commands\": \"codex\"\n}"
    );
}

#[test]
fn set_config_rejects_an_unknown_key_before_saving() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(set("api_key", "sk-secret"), &environment);
    assert_eq!(
        output.result.unwrap_err().to_string(),
        "Unknown config key 'api_key'. Allowed keys: base_url, hidden_commands"
    );
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn set_config_rejects_an_invalid_value_before_saving() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(set("base_url", "proxy.example.com"), &environment);
    assert_eq!(output.result.unwrap_err().to_string(), FULL_URL_MESSAGE);
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn get_config_prints_the_stored_value() {
    let home = TempDir::new().unwrap();
    write_config(home.path(), r#"{"base_url": "https://proxy.example.com"}"#);
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(
        ConfigCommand::Get {
            key: Some("base_url".to_owned()),
        },
        &environment,
    );
    output.result.unwrap();
    assert_eq!(output.stdout, "https://proxy.example.com\n");
}

#[test]
fn get_config_reports_a_key_that_is_not_stored() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(
        ConfigCommand::Get {
            key: Some(HIDDEN_COMMANDS_KEY.to_owned()),
        },
        &environment,
    );
    let error = output.result.unwrap_err();
    assert!(
        matches!(&error, ConfigError::KeyNotSet(key) if key.as_str() == HIDDEN_COMMANDS_KEY),
        "{error:?}"
    );
    assert_eq!(error.to_string(), "hidden_commands is not set");
    assert_eq!(output.stdout, "");
}

#[test]
fn get_config_lists_every_stored_entry_in_key_order() {
    let home = TempDir::new().unwrap();
    write_config(
        home.path(),
        r#"{"hidden_commands": "codex", "base_url": "https://proxy.example.com"}"#,
    );
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(ConfigCommand::Get { key: None }, &environment);
    output.result.unwrap();
    assert_eq!(
        output.stdout,
        "base_url = https://proxy.example.com\nhidden_commands = codex\n"
    );
}

#[test]
fn get_config_prints_no_config_set_for_an_empty_store() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(ConfigCommand::Get { key: None }, &environment);
    output.result.unwrap();
    assert_eq!(output.stdout, "(no config set)\n");
}

#[test]
fn unset_config_reports_a_key_that_was_not_set() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(
        ConfigCommand::Unset {
            key: HIDDEN_COMMANDS_KEY.to_owned(),
        },
        &environment,
    );
    output.result.unwrap();
    assert_eq!(output.stdout, "hidden_commands was not set\n");
    assert!(!home.path().join(".litellm").exists());
}

#[test]
fn unset_config_removes_the_key_and_saves() {
    let home = TempDir::new().unwrap();
    write_config(
        home.path(),
        r#"{"base_url": "https://proxy.example.com", "hidden_commands": "codex"}"#,
    );
    let environment = FakeEnvironment::with_home(home.path());
    let output = run(
        ConfigCommand::Unset {
            key: "base_url".to_owned(),
        },
        &environment,
    );
    output.result.unwrap();
    assert_eq!(
        output.stdout,
        format!(
            "Removed base_url from {}\n",
            config_path(home.path()).display()
        )
    );
    assert_eq!(
        fs::read_to_string(config_path(home.path())).unwrap(),
        "{\n  \"hidden_commands\": \"codex\"\n}"
    );
}

#[test]
fn load_config_reads_a_single_stored_value() {
    let home = TempDir::new().unwrap();
    write_config(home.path(), r#"{"base_url": "https://proxy.example.com"}"#);
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    assert_eq!(
        load_config(&environment, &mut warnings)
            .get("base_url")
            .cloned(),
        Some("https://proxy.example.com".to_owned())
    );
    assert!(warnings.is_empty());
}

#[test]
fn load_config_lookup_returns_none_for_an_absent_key() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut warnings = Vec::new();
    assert_eq!(
        load_config(&environment, &mut warnings)
            .get("base_url")
            .cloned(),
        None
    );
}

#[rstest]
#[case::absent(None, &[])]
#[case::empty(Some(""), &[])]
#[case::separators_only(Some(" , ,"), &[])]
#[case::trims_each_name(Some("codex, opencode"), &["codex", "opencode"])]
#[case::drops_duplicates(Some("codex,codex,opencode"), &["codex", "opencode"])]
#[case::sorts_names(Some("opencode,codex"), &["codex", "opencode"])]
fn parse_hidden_commands_returns_the_named_set(
    #[case] raw: Option<&str>,
    #[case] expected: &[&str],
) {
    let wanted: BTreeSet<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(parse_hidden_commands(raw), wanted);
}

#[test]
fn hidden_command_names_reads_the_stored_entry() {
    let home = TempDir::new().unwrap();
    write_config(home.path(), r#"{"hidden_commands": "opencode, codex"}"#);
    let environment = FakeEnvironment::with_home(home.path());
    assert_eq!(
        hidden_command_names(&environment),
        BTreeSet::from(["codex".to_owned(), "opencode".to_owned()])
    );
}

#[test]
fn hidden_command_names_returns_an_empty_set_when_unset() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    assert_eq!(hidden_command_names(&environment), BTreeSet::new());
}

#[test]
fn hidden_command_names_ignores_an_invalid_config_file() {
    let home = TempDir::new().unwrap();
    write_config(home.path(), "not json");
    let environment = FakeEnvironment::with_home(home.path());
    assert_eq!(hidden_command_names(&environment), BTreeSet::new());
}

#[rstest]
#[case::empty("", EMPTY_LIST_MESSAGE)]
#[case::separators_only(" , ", EMPTY_LIST_MESSAGE)]
fn normalize_hidden_commands_rejects_a_value_without_names(
    #[case] raw: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        normalize_hidden_commands(raw).unwrap_err().to_string(),
        expected
    );
}

#[test]
fn normalize_hidden_commands_rejects_a_name_with_a_space() {
    assert_eq!(
        normalize_hidden_commands("codex, open code")
            .unwrap_err()
            .to_string(),
        "hidden_commands entries must be single command names, without spaces"
    );
}

#[test]
fn normalize_hidden_commands_sorts_and_joins_names() {
    assert_eq!(
        normalize_hidden_commands(" opencode , codex ").unwrap(),
        "codex,opencode"
    );
}

#[rstest]
#[case::http("http://localhost:4000", "http://localhost:4000")]
#[case::https("https://proxy.example.com", "https://proxy.example.com")]
#[case::one_trailing_slash("https://proxy.example.com/", "https://proxy.example.com")]
#[case::every_trailing_slash("https://proxy.example.com///", "https://proxy.example.com")]
#[case::at_sign_in_path(
    "https://proxy.example.com/team@acme",
    "https://proxy.example.com/team@acme"
)]
#[case::backslash_after_scheme(r"https://\proxy.example.com", r"https://\proxy.example.com")]
#[case::port_and_path(
    "https://proxy.example.com:8443/v1",
    "https://proxy.example.com:8443/v1"
)]
#[case::keeps_the_given_spelling(
    "HTTPS://Proxy.Example.COM:8443/",
    "HTTPS://Proxy.Example.COM:8443"
)]
fn normalize_base_url_accepts_and_trims(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(normalize_base_url(value).unwrap(), expected);
}

#[rstest]
#[case::space("https://proxy.example.com/a b")]
#[case::tab("https://proxy.example.com/\t")]
#[case::newline("https://proxy.example.com/\n")]
#[case::nul("https://proxy.example.com/\0")]
#[case::delete("https://proxy.example.com/\u{7f}")]
fn normalize_base_url_rejects_control_characters(#[case] value: &str) {
    assert_eq!(
        normalize_base_url(value).unwrap_err().to_string(),
        CONTROL_CHARACTER_MESSAGE
    );
}

#[rstest]
#[case::bare_host("proxy.example.com")]
#[case::wrong_scheme("ftp://proxy.example.com")]
#[case::empty_host("http://")]
#[case::three_slashes("http:///proxy.example.com")]
#[case::missing_slashes("http:proxy.example.com")]
#[case::invalid_port("http://proxy.example.com:99999")]
fn normalize_base_url_rejects_a_missing_http_host(#[case] value: &str) {
    assert_eq!(
        normalize_base_url(value).unwrap_err().to_string(),
        FULL_URL_MESSAGE
    );
}

#[rstest]
#[case::query("https://proxy.example.com/?page=1")]
#[case::fragment("https://proxy.example.com/#top")]
fn normalize_base_url_rejects_a_query_or_fragment(#[case] value: &str) {
    assert_eq!(
        normalize_base_url(value).unwrap_err().to_string(),
        QUERY_MESSAGE
    );
}

#[rstest]
#[case::user_and_password("https://user:pass@proxy.example.com")]
#[case::user_only("https://user@proxy.example.com")]
#[case::empty_user("https://@proxy.example.com")]
#[case::backslash_before_userinfo(r"https://DOMAIN\user@proxy.example.com")]
fn normalize_base_url_rejects_credentials(#[case] value: &str) {
    assert_eq!(
        normalize_base_url(value).unwrap_err().to_string(),
        CREDENTIALS_MESSAGE
    );
}

#[rstest]
#[case::control_before_scheme(" https://user:pass@proxy.example.com", CONTROL_CHARACTER_MESSAGE)]
#[case::scheme_before_query("ftp://user:pass@proxy.example.com?page=1", FULL_URL_MESSAGE)]
#[case::query_before_credentials("https://user:pass@proxy.example.com#top", QUERY_MESSAGE)]
fn normalize_base_url_checks_in_litellm_order(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(normalize_base_url(value).unwrap_err().to_string(), expected);
}
