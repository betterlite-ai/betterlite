#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::worker_options_reload::{
    LogConfig, LoopType, ReloadSettings, UvicornSettings,
};
use betterlite_db::database_diagnostics::level::LogLevel;
use rstest::rstest;
use tempfile::TempDir;

use crate::worker_options::dev_reload::{
    ReloadError, configure_dev_reload, patch_reload_extra_paths, resolve_reload_options,
};
use crate::worker_options::loop_type::{SysPlatform, resolve_loop_type};
use crate::worker_options::uvicorn_settings::{
    UvicornOverrides, apply_max_requests_jitter, resolve_uvicorn_settings,
};

fn base() -> UvicornSettings {
    UvicornSettings::default()
}

fn base_with_log(log_config: LogConfig) -> UvicornSettings {
    UvicornSettings {
        log_config,
        ..base()
    }
}

fn base_with_healthcheck(timeout_worker_healthcheck: Option<i64>) -> UvicornSettings {
    UvicornSettings {
        timeout_worker_healthcheck,
        ..base()
    }
}

fn base_with_limits(
    limit_max_requests: Option<i64>,
    limit_max_requests_jitter: Option<i64>,
) -> UvicornSettings {
    UvicornSettings {
        limit_max_requests,
        limit_max_requests_jitter,
        ..base()
    }
}

fn no_overrides() -> UvicornOverrides {
    UvicornOverrides::default()
}

fn interface_override(host: &str, port: u16, keepalive_timeout: i64) -> UvicornOverrides {
    UvicornOverrides {
        host: Some(host.to_owned()),
        port: Some(port),
        log_config: None,
        keepalive_timeout: Some(keepalive_timeout),
        timeout_worker_healthcheck: None,
        max_requests: None,
        max_requests_jitter: None,
    }
}

fn log_override(path: Option<&Path>) -> UvicornOverrides {
    UvicornOverrides {
        host: None,
        port: None,
        log_config: path.map(Path::to_path_buf),
        keepalive_timeout: None,
        timeout_worker_healthcheck: None,
        max_requests: None,
        max_requests_jitter: None,
    }
}

fn healthcheck_override(timeout_worker_healthcheck: Option<i64>) -> UvicornOverrides {
    UvicornOverrides {
        host: None,
        port: None,
        log_config: None,
        keepalive_timeout: None,
        timeout_worker_healthcheck,
        max_requests: None,
        max_requests_jitter: None,
    }
}

fn resolved(
    base: &UvicornSettings,
    overrides: &UvicornOverrides,
) -> (UvicornSettings, Vec<String>) {
    let mut warnings = Vec::new();
    let mut warn = |message: &str| warnings.push(message.to_owned());
    let settings = resolve_uvicorn_settings(base, overrides, &mut warn);
    (settings, warnings)
}

fn temporary_directory() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = fs::canonicalize(directory.path()).expect("canonical root");
    (directory, root)
}

fn configured(cwd: &Path, config: Option<&Path>) -> (ReloadSettings, Vec<String>) {
    let mut settings = ReloadSettings::default();
    let mut warnings = Vec::new();
    let mut warn = |message: &str| warnings.push(message.to_owned());
    configure_dev_reload(&mut settings, cwd, config, &mut warn).expect("dev reload configuration");
    (settings, warnings)
}

#[test]
fn worker_options_uvicorn_settings_take_the_litellm_defaults() {
    let (settings, warnings) = resolved(&base(), &no_overrides());
    assert_eq!(settings.host, "0.0.0.0");
    assert_eq!(settings.port, 4000);
    assert!(!settings.server_header);
    assert_eq!(settings.log_config, LogConfig::Builtin);
    assert_eq!(settings.timeout_keep_alive, None);
    assert_eq!(settings.timeout_worker_healthcheck, None);
    assert_eq!(settings.limit_max_requests, None);
    assert_eq!(settings.limit_max_requests_jitter, None);
    assert_eq!(
        settings.loop_type,
        resolve_loop_type(SysPlatform::current())
    );
    assert!(warnings.is_empty());
}

#[test]
fn worker_options_uvicorn_settings_flags_win_over_the_config() {
    let mut section = base();
    section.host = "10.0.0.1".to_owned();
    section.port = 8000;
    section.timeout_keep_alive = Some(5);
    let (settings, _warnings) = resolved(&section, &interface_override("127.0.0.1", 9000, 12));
    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.port, 9000);
    assert_eq!(settings.timeout_keep_alive, Some(12));
}

#[test]
fn worker_options_uvicorn_settings_keep_the_config_when_no_flag_is_given() {
    let mut section = base();
    section.host = "10.0.0.1".to_owned();
    section.port = 8000;
    section.timeout_keep_alive = Some(5);
    let (settings, _warnings) = resolved(&section, &no_overrides());
    assert_eq!(settings.host, "10.0.0.1");
    assert_eq!(settings.port, 8000);
    assert_eq!(settings.timeout_keep_alive, Some(5));
}

#[test]
fn worker_options_uvicorn_settings_always_disable_the_server_header() {
    let mut section = base();
    section.server_header = true;
    let (settings, _warnings) = resolved(&section, &no_overrides());
    assert!(!settings.server_header);
}

#[rstest]
#[case::flag_over_json(
    LogConfig::Json,
    Some(Path::new("/etc/logging.yaml")),
    LogConfig::File(PathBuf::from("/etc/logging.yaml"))
)]
#[case::flag_over_level(
    LogConfig::Level(LogLevel::Debug),
    Some(Path::new("/etc/logging.yaml")),
    LogConfig::File(PathBuf::from("/etc/logging.yaml"))
)]
#[case::json_without_a_flag(LogConfig::Json, None, LogConfig::Json)]
#[case::level_without_a_flag(
    LogConfig::Level(LogLevel::Warning),
    None,
    LogConfig::Level(LogLevel::Warning)
)]
#[case::builtin_without_a_flag(LogConfig::Builtin, None, LogConfig::Builtin)]
fn worker_options_uvicorn_settings_resolve_the_log_config(
    #[case] log_config: LogConfig,
    #[case] flag: Option<&Path>,
    #[case] expected: LogConfig,
) {
    let (settings, _warnings) = resolved(&base_with_log(log_config), &log_override(flag));
    assert_eq!(settings.log_config, expected);
}

#[rstest]
#[case::from_config(None, Some(45), Some(45), 1)]
#[case::from_flag(Some(60), Some(45), Some(60), 1)]
#[case::from_flag_over_unset(Some(60), None, Some(60), 1)]
#[case::unset(None, None, None, 0)]
fn worker_options_uvicorn_settings_warn_about_the_healthcheck_timeout(
    #[case] flag: Option<i64>,
    #[case] configured: Option<i64>,
    #[case] expected: Option<i64>,
    #[case] warnings: usize,
) {
    let (settings, captured) = resolved(
        &base_with_healthcheck(configured),
        &healthcheck_override(flag),
    );
    assert_eq!(settings.timeout_worker_healthcheck, expected);
    assert_eq!(captured.len(), warnings);
    assert!(
        captured
            .iter()
            .all(|warning| warning.contains("--timeout_worker_healthcheck"))
    );
}

#[test]
fn worker_options_uvicorn_settings_carry_the_request_limit_and_its_jitter() {
    let (settings, warnings) = resolved(
        &base_with_limits(Some(100), Some(10)),
        &UvicornOverrides {
            max_requests: Some(500),
            ..no_overrides()
        },
    );
    assert_eq!(settings.limit_max_requests, Some(500));
    assert_eq!(settings.limit_max_requests_jitter, Some(10));
    assert!(warnings.is_empty());
}

#[test]
fn worker_options_uvicorn_settings_take_a_flag_jitter_over_the_config() {
    let (settings, warnings) = resolved(
        &base_with_limits(Some(100), Some(10)),
        &UvicornOverrides {
            max_requests_jitter: Some(25),
            ..no_overrides()
        },
    );
    assert_eq!(settings.limit_max_requests, Some(100));
    assert_eq!(settings.limit_max_requests_jitter, Some(25));
    assert!(warnings.is_empty());
}

#[test]
fn worker_options_jitter_is_applied_next_to_a_limit() {
    let (settings, warnings) = resolved(&base_with_limits(Some(100), Some(10)), &no_overrides());
    assert_eq!(settings.limit_max_requests_jitter, Some(10));
    assert!(warnings.is_empty());
}

#[test]
fn worker_options_jitter_without_a_limit_warns() {
    let (settings, warnings) = resolved(&base_with_limits(None, Some(10)), &no_overrides());
    assert_eq!(settings.limit_max_requests_jitter, None);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("--max_requests_before_restart_jitter"));
}

#[test]
fn worker_options_jitter_absent_warns_nothing() {
    let (settings, warnings) = resolved(&base_with_limits(None, None), &no_overrides());
    assert_eq!(settings.limit_max_requests_jitter, None);
    assert!(warnings.is_empty());
}

#[test]
fn worker_options_jitter_helper_applies_and_warns_on_its_own() {
    let mut warnings = Vec::new();
    let mut warn = |message: &str| warnings.push(message.to_owned());
    assert_eq!(
        apply_max_requests_jitter(Some(100), Some(7), &mut warn),
        Some(7)
    );
    assert_eq!(apply_max_requests_jitter(None, Some(7), &mut warn), None);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn reload_options_watch_the_current_directory_without_a_config() {
    let (_directory, root) = temporary_directory();
    let options = resolve_reload_options(&root, None).expect("reload options");
    assert!(options.reload);
    assert_eq!(options.reload_dirs, vec![root]);
    assert_eq!(
        options.reload_includes,
        vec!["*.py".to_owned(), ".env".to_owned()]
    );
    assert!(options.extra_paths.is_empty());
    assert!(!options.env_hot_reload);
}

#[test]
fn reload_options_add_the_config_parent_and_its_basename() {
    let (_directory, root) = temporary_directory();
    let config_dir = root.join("configs");
    fs::create_dir(&config_dir).expect("config directory");
    let config = config_dir.join("proxy.yaml");
    fs::write(&config, "{}").expect("config file");
    let options = resolve_reload_options(&root, Some(config.as_path())).expect("reload options");
    assert_eq!(options.reload_dirs, vec![root, config_dir]);
    assert_eq!(
        options.reload_includes,
        vec![
            "*.py".to_owned(),
            ".env".to_owned(),
            "proxy.yaml".to_owned()
        ]
    );
}

#[test]
fn reload_options_do_not_repeat_a_config_in_the_watched_directory() {
    let (_directory, root) = temporary_directory();
    let config = root.join("proxy.yaml");
    fs::write(&config, "{}").expect("config file");
    let options = resolve_reload_options(&root, Some(config.as_path())).expect("reload options");
    assert_eq!(options.reload_dirs, vec![root]);
    assert_eq!(
        options.reload_includes,
        vec![
            "*.py".to_owned(),
            ".env".to_owned(),
            "proxy.yaml".to_owned()
        ]
    );
}

#[test]
fn reload_options_resolve_a_relative_config_against_the_watched_directory() {
    let (_directory, root) = temporary_directory();
    fs::write(root.join("proxy.yaml"), "{}").expect("config file");
    let options = resolve_reload_options(&root, Some(Path::new("proxy.yaml"))).expect("options");
    assert_eq!(options.reload_dirs, vec![root]);
    assert_eq!(
        options.reload_includes,
        vec![
            "*.py".to_owned(),
            ".env".to_owned(),
            "proxy.yaml".to_owned()
        ]
    );
}

#[cfg(unix)]
#[test]
fn reload_options_follow_a_symlinked_watch_directory_to_its_target() {
    use std::os::unix::fs::symlink;

    let (_directory, root) = temporary_directory();
    let target = root.join("target");
    let link = root.join("link");
    fs::create_dir(&target).expect("target directory");
    symlink(&target, &link).expect("symlink");
    let options = resolve_reload_options(&link, None).expect("reload options");
    assert_eq!(options.reload_dirs, vec![target]);
}

#[cfg(unix)]
#[test]
fn reload_options_reject_a_config_name_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (_directory, root) = temporary_directory();
    let config = root.join(OsStr::from_bytes(b"proxy-\xff.yaml"));
    fs::write(&config, "{}").expect("config file");
    assert!(matches!(
        resolve_reload_options(&root, Some(config.as_path())),
        Err(ReloadError::FileName { .. })
    ));
}

#[test]
fn reload_patch_records_the_resolved_paths() {
    let (_directory, root) = temporary_directory();
    let config = root.join("proxy.yaml");
    let env_file = root.join(".env");
    fs::write(&config, "{}").expect("config file");
    let mut settings = ReloadSettings::default();
    let patched = patch_reload_extra_paths(
        &mut settings,
        &root,
        &[Some(config.as_path()), Some(env_file.as_path())],
    )
    .expect("patched paths");
    assert!(patched);
    assert_eq!(settings.extra_paths, BTreeSet::from([config, env_file]));
}

#[test]
fn reload_patch_resolves_a_relative_path_against_the_cwd() {
    let (_directory, root) = temporary_directory();
    let mut settings = ReloadSettings::default();
    let patched =
        patch_reload_extra_paths(&mut settings, &root, &[Some(Path::new(".env"))]).expect("paths");
    assert!(patched);
    assert_eq!(settings.extra_paths, BTreeSet::from([root.join(".env")]));
}

#[test]
fn reload_patch_is_idempotent() {
    let (_directory, root) = temporary_directory();
    let env_file = root.join(".env");
    let mut settings = ReloadSettings::default();
    patch_reload_extra_paths(&mut settings, &root, &[Some(env_file.as_path())])
        .expect("first patch");
    patch_reload_extra_paths(&mut settings, &root, &[Some(env_file.as_path())])
        .expect("second patch");
    assert_eq!(settings.extra_paths, BTreeSet::from([env_file]));
}

#[test]
fn reload_patch_with_no_paths_returns_false() {
    let (_directory, root) = temporary_directory();
    let mut settings = ReloadSettings::default();
    let patched = patch_reload_extra_paths(&mut settings, &root, &[None]).expect("patched paths");
    assert!(!patched);
    assert!(settings.extra_paths.is_empty());
}

#[test]
fn dev_reload_warns_that_dot_env_wins_after_a_reload() {
    let (_directory, root) = temporary_directory();
    let (_settings, warnings) = configured(&root, None);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains(".env values win over shell-exported environment variables"));
}

#[test]
fn dev_reload_turns_on_the_hot_reload_setting() {
    let (_directory, root) = temporary_directory();
    let (settings, _warnings) = configured(&root, None);
    assert!(settings.reload);
    assert!(settings.env_hot_reload);
}

#[test]
fn dev_reload_watches_the_config_and_the_dot_env() {
    let (_directory, root) = temporary_directory();
    let config_dir = root.join("configs");
    fs::create_dir(&config_dir).expect("config directory");
    let config = config_dir.join("proxy.yaml");
    fs::write(&config, "{}").expect("config file");
    let (settings, _warnings) = configured(&root, Some(config.as_path()));
    assert_eq!(settings.reload_dirs, vec![root.clone(), config_dir]);
    assert_eq!(
        settings.reload_includes,
        vec![
            "*.py".to_owned(),
            ".env".to_owned(),
            "proxy.yaml".to_owned()
        ]
    );
    assert_eq!(
        settings.extra_paths,
        BTreeSet::from([config, root.join(".env")])
    );
}

#[test]
fn dev_reload_without_a_config_watches_only_the_dot_env() {
    let (_directory, root) = temporary_directory();
    let (settings, _warnings) = configured(&root, None);
    assert_eq!(settings.extra_paths, BTreeSet::from([root.join(".env")]));
}

#[test]
fn dev_reload_is_repeatable() {
    let (_directory, root) = temporary_directory();
    let mut settings = ReloadSettings::default();
    let mut warnings = Vec::new();
    let mut warn = |message: &str| warnings.push(message.to_owned());
    configure_dev_reload(&mut settings, &root, None, &mut warn).expect("first configuration");
    configure_dev_reload(&mut settings, &root, None, &mut warn).expect("second configuration");
    assert_eq!(settings.extra_paths, BTreeSet::from([root.join(".env")]));
    assert_eq!(warnings.len(), 2);
}

#[rstest]
#[case::windows(SysPlatform::Win32)]
#[case::cygwin(SysPlatform::Cygwin)]
#[case::cli(SysPlatform::Cli)]
fn loop_type_is_absent_on_platforms_python_lacks_uvloop_for(#[case] platform: SysPlatform) {
    assert_eq!(resolve_loop_type(platform), None);
}

#[test]
fn loop_type_is_uvloop_on_the_other_platforms() {
    assert_eq!(
        resolve_loop_type(SysPlatform::Other),
        Some(LoopType::Uvloop)
    );
}

#[test]
fn loop_type_of_the_current_platform_follows_the_windows_cfg() {
    let expected = if cfg!(windows) {
        None
    } else {
        Some(LoopType::Uvloop)
    };
    assert_eq!(resolve_loop_type(SysPlatform::current()), expected);
}
