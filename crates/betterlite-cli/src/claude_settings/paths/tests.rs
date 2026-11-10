#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

use super::{
    CLAUDE_CONFIG_DIR_ENV, SettingsFileOwner, claude_settings_path, claude_settings_paths,
    configure_state_path, refuse_while_owned, settings_file_owners,
};
use crate::claude_settings::error::ClaudeSettingsError;

struct FakeEnvironment {
    variables: BTreeMap<String, String>,
    home: Option<PathBuf>,
}

impl FakeEnvironment {
    fn new(home: Option<&Path>, variables: &[(&str, &str)]) -> Self {
        Self {
            variables: variables
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            home: home.map(Path::to_path_buf),
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

fn home_environment(variables: &[(&str, &str)]) -> FakeEnvironment {
    FakeEnvironment::new(Some(Path::new("/home/tester")), variables)
}

#[test]
fn claude_settings_path_defaults_to_the_home_settings() {
    let environment = home_environment(&[]);
    assert_eq!(
        claude_settings_path(&environment),
        Some(PathBuf::from("/home/tester/.claude/settings.json"))
    );
}

#[rstest]
#[case::absolute("/opt/claude", "/opt/claude/settings.json")]
#[case::trailing_slash("/opt/claude/", "/opt/claude/settings.json")]
#[case::relative("config", "config/settings.json")]
fn claude_settings_path_follows_the_config_dir(#[case] config_dir: &str, #[case] expected: &str) {
    let environment = home_environment(&[(CLAUDE_CONFIG_DIR_ENV, config_dir)]);
    assert_eq!(
        claude_settings_path(&environment),
        Some(PathBuf::from(expected))
    );
}

#[rstest]
#[case::tilde("~/dotfiles/claude", "/home/tester/dotfiles/claude/settings.json")]
#[case::bare_tilde("~", "/home/tester/settings.json")]
fn claude_settings_path_expands_a_leading_tilde(#[case] config_dir: &str, #[case] expected: &str) {
    let environment = home_environment(&[(CLAUDE_CONFIG_DIR_ENV, config_dir)]);
    assert_eq!(
        claude_settings_path(&environment),
        Some(PathBuf::from(expected))
    );
}

#[test]
fn claude_settings_path_leaves_another_users_tilde_alone() {
    let environment = home_environment(&[(CLAUDE_CONFIG_DIR_ENV, "~other/claude")]);
    assert_eq!(
        claude_settings_path(&environment),
        Some(PathBuf::from("~other/claude/settings.json"))
    );
}

#[test]
fn claude_settings_path_treats_an_empty_config_dir_as_unset() {
    let environment = home_environment(&[(CLAUDE_CONFIG_DIR_ENV, "")]);
    assert_eq!(
        claude_settings_path(&environment),
        Some(PathBuf::from("/home/tester/.claude/settings.json"))
    );
}

#[test]
fn claude_settings_path_without_a_home_is_none() {
    let environment = FakeEnvironment::new(None, &[]);
    assert_eq!(claude_settings_path(&environment), None);
}

#[test]
fn claude_settings_path_without_a_home_still_takes_an_absolute_config_dir() {
    let environment = FakeEnvironment::new(None, &[(CLAUDE_CONFIG_DIR_ENV, "/opt/claude")]);
    assert_eq!(
        claude_settings_path(&environment),
        Some(PathBuf::from("/opt/claude/settings.json"))
    );
}

#[test]
fn claude_settings_path_without_a_home_cannot_expand_a_tilde() {
    let environment = FakeEnvironment::new(None, &[(CLAUDE_CONFIG_DIR_ENV, "~/claude")]);
    assert_eq!(claude_settings_path(&environment), None);
}

#[test]
fn settings_file_owners_guard_the_default_settings_file() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    let owners = settings_file_owners(&paths.settings, &paths).unwrap();
    assert_eq!(
        owners,
        vec![
            SettingsFileOwner {
                backup_path: home
                    .path()
                    .join(".litellm")
                    .join("claude_settings_backup.json"),
                start_command: "lite up",
                stop_command: "lite down",
            },
            SettingsFileOwner {
                backup_path: home
                    .path()
                    .join(".litellm")
                    .join("autorouter")
                    .join("claude_settings_backup.json"),
                start_command: "lite autoroute start",
                stop_command: "lite autoroute stop",
            },
        ]
    );
}

#[test]
fn settings_file_owners_guard_nothing_else() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    let other = home.path().join("config").join("settings.json");
    assert_eq!(settings_file_owners(&other, &paths).unwrap(), Vec::new());
}

#[cfg(unix)]
#[test]
fn settings_file_owners_follow_a_symlinked_settings_file() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.settings.parent().unwrap()).unwrap();
    fs::create_dir_all(home.path().join("dotfiles")).unwrap();
    fs::write(home.path().join("dotfiles").join("settings.json"), "{}").unwrap();
    symlink(
        home.path().join("dotfiles").join("settings.json"),
        &paths.settings,
    )
    .unwrap();
    let link = home.path().join("elsewhere.json");
    symlink(&paths.settings, &link).unwrap();
    assert_eq!(
        settings_file_owners(&paths.settings, &paths).unwrap().len(),
        2
    );
    assert_eq!(settings_file_owners(&link, &paths).unwrap().len(), 2);
}

#[test]
fn configure_state_path_default_is_shared_by_the_default_settings_file() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    assert_eq!(
        configure_state_path(&paths.settings, &paths).unwrap(),
        home.path()
            .join(".litellm")
            .join("claude_configure_state.json")
    );
}

#[test]
fn configure_state_path_keys_another_settings_file_by_its_resolved_path() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    let config_dir = home.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    let settings = config_dir.join("settings.json");
    fs::write(&settings, "{}").unwrap();
    let receipt = configure_state_path(&settings, &paths).unwrap();
    let resolved = fs::canonicalize(&settings).unwrap();
    let digest = hex::encode(Sha256::digest(resolved.as_os_str().as_encoded_bytes()));
    assert_eq!(
        receipt,
        home.path()
            .join(".litellm")
            .join("claude_configure_state")
            .join(format!("{digest}.json"))
    );
    let respelled = config_dir.join("..").join("config").join("settings.json");
    assert_eq!(configure_state_path(&respelled, &paths).unwrap(), receipt);
    assert_ne!(
        configure_state_path(&config_dir.join("other.json"), &paths).unwrap(),
        receipt
    );
}

#[test]
fn refuse_while_owned_names_the_up_backup() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.backup.parent().unwrap()).unwrap();
    fs::write(&paths.backup, "{}").unwrap();
    let owners = settings_file_owners(&paths.settings, &paths).unwrap();
    let error = refuse_while_owned(&paths.settings, &owners).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("lite up"));
    assert!(message.contains("lite down"));
    assert!(message.contains(&paths.backup.display().to_string()));
    assert!(message.contains(&paths.settings.display().to_string()));
    assert!(matches!(error, ClaudeSettingsError::Owned { .. }));
}

#[test]
fn refuse_while_owned_names_the_autoroute_backup() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.autoroute_backup.parent().unwrap()).unwrap();
    fs::write(&paths.autoroute_backup, "{}").unwrap();
    let owners = settings_file_owners(&paths.settings, &paths).unwrap();
    let error = refuse_while_owned(&paths.settings, &owners).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("lite autoroute start"));
    assert!(message.contains("lite autoroute stop"));
    assert!(message.contains(&paths.autoroute_backup.display().to_string()));
}

#[test]
fn refuse_while_owned_reports_the_up_owner_before_the_autoroute_one() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.backup.parent().unwrap()).unwrap();
    fs::create_dir_all(paths.autoroute_backup.parent().unwrap()).unwrap();
    fs::write(&paths.backup, "{}").unwrap();
    fs::write(&paths.autoroute_backup, "{}").unwrap();
    let owners = settings_file_owners(&paths.settings, &paths).unwrap();
    let error = refuse_while_owned(&paths.settings, &owners).unwrap_err();
    assert!(error.to_string().contains("lite up"));
    assert!(!error.to_string().contains("lite autoroute start"));
}

#[test]
fn refuse_while_owned_allows_a_write_with_no_backup() {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    let owners = settings_file_owners(&paths.settings, &paths).unwrap();
    refuse_while_owned(&paths.settings, &owners).unwrap();
}

#[test]
fn resolve_settings_path_reports_an_unresolvable_path() {
    let error = super::resolve_settings_path(Path::new("")).unwrap_err();
    assert!(matches!(error, ClaudeSettingsError::Resolve { .. }));
}
