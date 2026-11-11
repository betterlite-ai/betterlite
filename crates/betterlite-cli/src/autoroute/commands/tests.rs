#![cfg(test)]

mod support;

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::{FileAccess, LocalFiles, PrivateFiles};
use indexmap::IndexMap;
use tempfile::TempDir;

use self::support::{
    BackupOrderFiles, FakeClock, FakeSignals, Fixture, LAUNCHED_PID, PERSISTED_KEY, default_fixture,
};
use super::args::{AutorouteCommand, StartArgs};
use super::deprecation_notice;
use super::error::CommandsError;
use super::teardown::{TeardownRequest, run_teardown};
use crate::autoroute::commands::liveliness::{HttpLiveliness, LIVELINESS_TIMEOUT_SECONDS};
use crate::autoroute::config::model::RawModelListing;
use crate::autoroute::paths::pid_record_path;
use crate::autoroute::process::control::TermSignal;
use crate::autoroute::process::liveliness::LivelinessProbe as _;
use crate::autoroute::process::pid_record::{PidRecord, write_pid_record};
use crate::autoroute::process::runtime::GATEWAY_INSTALL_STEP;
use crate::autoroute::wizard::error::WizardError;
use crate::claude_settings::SettingsDocument;
use crate::claude_settings::paths::claude_settings_paths;
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Http, HttpBody, HttpResponse};
use crate::settings_backup::record::BackupRecord;
use crate::settings_backup::write::write_backup;

const PORT: u16 = 5483;

fn record(pid: u32) -> PidRecord {
    PidRecord {
        pid,
        port: PORT,
        config_path: PathBuf::from("/home/tester/.litellm/autorouter/config.yaml"),
        log_path: PathBuf::from("/home/tester/.litellm/autorouter/proxy.log"),
    }
}

fn token(fixture: &Fixture) -> Option<String> {
    let settings = fixture.snapshot().settings?;
    let env = settings.get("env")?;
    env.get("ANTHROPIC_AUTH_TOKEN")?.as_str().map(str::to_owned)
}

fn model_field(path: &Path) -> Option<String> {
    let document: SettingsDocument =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    document.get("model")?.as_str().map(str::to_owned)
}

fn chat_model(id: &str) -> RawModelListing {
    RawModelListing {
        id: id.to_owned(),
        mode: "chat".to_owned(),
    }
}

#[test]
fn port_4000_rejected_before_the_port_probe_runs() {
    let mut fixture = default_fixture();
    let error = fixture.start(4000).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("4000"), "{text}");
    assert!(text.contains("silently rebinds"), "{text}");
    assert!(fixture.port_probe.probed.lock().unwrap().is_empty());
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn port_in_use_refusal_when_the_probe_reports_the_port_unavailable() {
    let mut fixture = default_fixture();
    fixture.port_probe.available = false;
    let error = fixture.start(5001).unwrap_err();
    assert!(error.to_string().contains("already in use"), "{error}");
    assert_eq!(*fixture.port_probe.probed.lock().unwrap(), [5001]);
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn already_running_detection_names_the_stop_command() {
    let mut fixture = default_fixture();
    write_pid_record(&fixture.pid_record_path(), &record(999)).unwrap();
    fixture.signals.alive.lock().unwrap().push(999);
    let error = fixture.start(PORT).unwrap_err();
    assert!(error.to_string().contains("already running"), "{error}");
    assert!(error.to_string().contains("lite autoroute stop"), "{error}");
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn start_fails_on_a_corrupt_pid_record() {
    let mut fixture = default_fixture();
    fs::write(fixture.pid_record_path(), "{not json").unwrap();
    let error = fixture.start(PORT).unwrap_err();
    assert!(
        error.to_string().contains("invalid or unexpected JSON"),
        "{error}"
    );
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn backup_exists_refusal_before_the_port_checks() {
    let mut fixture = default_fixture();
    fs::write(fixture.backup_path(), "{}\n").unwrap();
    let error = fixture.start(PORT).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("already exists"), "{text}");
    assert!(text.contains("crashed without cleanup"), "{text}");
    assert!(fixture.port_probe.probed.lock().unwrap().is_empty());
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn start_refuses_without_a_configuration() {
    let mut fixture = default_fixture();
    fs::remove_file(fixture.config_path()).unwrap();
    let error = fixture.start(PORT).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("No config found. Run `lite autoroute configure` first."),
        "{error}"
    );
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn gateway_runtime_missing_refusal_names_the_install_step() {
    let mut fixture = default_fixture();
    fixture.gateway.executable = None;
    let error = fixture.start(PORT).unwrap_err();
    assert!(error.to_string().contains(GATEWAY_INSTALL_STEP), "{error}");
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn master_key_provisioning_reuses_the_persisted_key() {
    let mut fixture = default_fixture();
    let before = fs::read_to_string(fixture.config_path()).unwrap();
    fixture.start(PORT).unwrap();
    assert_eq!(fixture.entropy.filled, 0);
    assert_eq!(fs::read_to_string(fixture.config_path()).unwrap(), before);
    assert_eq!(token(&fixture).unwrap(), PERSISTED_KEY);
}

#[test]
fn master_key_provisioning_mints_and_rewrites_the_config_when_absent() {
    let mut fixture = default_fixture();
    fixture.entropy.pattern = (1_u8..=32).collect();
    fixture.write_config("model_list: []\nlitellm_settings:\n  master_key: sk-elsewhere\n");
    fixture.start(PORT).unwrap();
    let expected = URL_SAFE_NO_PAD.encode(fixture.entropy.pattern.clone());
    assert_eq!(fixture.entropy.filled, 32);
    let text = fs::read_to_string(fixture.config_path()).unwrap();
    let document: IndexMap<String, JsonValue> = serde_saphyr::from_str(&text).unwrap();
    assert_eq!(
        document["general_settings"]["master_key"].as_str(),
        Some(expected.as_str())
    );
    assert_eq!(
        document["litellm_settings"]["master_key"].as_str(),
        Some("sk-elsewhere")
    );
    assert_eq!(token(&fixture).unwrap(), expected);
}

#[test]
fn master_key_provisioning_treats_a_blank_key_as_absent() {
    let mut fixture = default_fixture();
    fixture.entropy.pattern = (1_u8..=32).collect();
    fixture.write_config("model_list: []\ngeneral_settings:\n  master_key: \"   \"\n");
    fixture.start(PORT).unwrap();
    let expected = URL_SAFE_NO_PAD.encode(fixture.entropy.pattern.clone());
    assert_eq!(token(&fixture).unwrap(), expected);
}

#[test]
fn master_key_provisioning_rejects_a_corrupt_config() {
    let mut fixture = default_fixture();
    fixture.write_config("model_list: [\n");
    let error = fixture.start(PORT).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("is empty or corrupt"), "{text}");
    assert!(text.contains("lite autoroute configure"), "{text}");
    assert!(fixture.launcher.requests.lock().unwrap().is_empty());
}

#[test]
fn start_writes_the_pid_record_and_restores_settings_after_the_wait() {
    let mut fixture = default_fixture();
    let settings = fixture.settings_path();
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"model":"old"}"#).unwrap();
    fixture.start(PORT).unwrap();
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.record.unwrap().pid, LAUNCHED_PID);
    assert_eq!(
        snapshot.settings.as_ref().unwrap()["env"]["ANTHROPIC_BASE_URL"].as_str(),
        Some("http://127.0.0.1:5483")
    );
    assert!(snapshot.backup.is_some());
    assert_eq!(fixture.signals.sent(), [(LAUNCHED_PID, TermSignal::Term)]);
    assert_eq!(model_field(&settings).as_deref(), Some("old"));
    assert!(!fixture.backup_path().exists());
    assert!(!fixture.pid_record_path().exists());
    let stdout = fixture.stdout_text();
    assert!(
        stdout.contains("ephemeral auto-router proxy up at http://127.0.0.1:5483"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Stopped ephemeral proxy and restored Claude Code settings."),
        "{stdout}"
    );
}

#[test]
fn start_launches_the_gateway_with_the_config_port_and_loopback_host() {
    let mut fixture = default_fixture();
    fixture.start(6000).unwrap();
    let recorded = fixture.launcher.requests.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].executable,
        PathBuf::from("/usr/local/bin/betterlite")
    );
    assert_eq!(
        recorded[0].arguments,
        [
            "--config",
            fixture.config_path().to_str().unwrap(),
            "--port",
            "6000",
            "--host",
            "127.0.0.1"
        ]
    );
    assert!(recorded[0].log_path.ends_with("proxy.log"));
}

#[test]
fn backup_restore_order_writes_the_backup_before_the_merged_settings() {
    let mut fixture = default_fixture();
    let settings = fixture.settings_path();
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"model":"old"}"#).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let files_access = BackupOrderFiles {
        inner: LocalFiles,
        backup_path: fixture.backup_path(),
        backup_seen: Arc::clone(&seen),
    };
    let shared: Arc<dyn FileAccess> = Arc::new(files_access);
    fixture.files = PrivateFiles::from(shared);
    fixture.start(PORT).unwrap();
    assert_eq!(seen.lock().unwrap().first(), Some(&true));
    assert_eq!(model_field(&settings).as_deref(), Some("old"));
    assert!(!fixture.backup_path().exists());
}

struct TeardownWorld {
    _home: TempDir,
    settings: PathBuf,
    backup: PathBuf,
    pid_record: PathBuf,
    signals: FakeSignals,
    clock: FakeClock,
    files: PrivateFiles,
}

fn teardown_world() -> TeardownWorld {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.settings.parent().unwrap()).unwrap();
    fs::write(&paths.settings, r#"{"model":"old"}"#).unwrap();
    let original: SettingsDocument = serde_json::from_str(r#"{"model":"old"}"#).unwrap();
    let files = PrivateFiles::default();
    write_backup(
        &BackupRecord {
            existed: true,
            content: Some(original),
        },
        &paths.autoroute_backup,
        &files,
    )
    .unwrap();
    let pid_record = pid_record_path(home.path());
    write_pid_record(&pid_record, &record(LAUNCHED_PID)).unwrap();
    TeardownWorld {
        _home: home,
        settings: paths.settings,
        backup: paths.autoroute_backup,
        pid_record,
        signals: FakeSignals::default(),
        clock: FakeClock::default(),
        files,
    }
}

impl TeardownWorld {
    fn request<'request>(
        &'request self,
        stdout: &'request mut dyn Write,
        stderr: &'request mut dyn Write,
    ) -> TeardownRequest<'request> {
        TeardownRequest {
            settings: &self.settings,
            backup: &self.backup,
            pid_record: &self.pid_record,
            pid: LAUNCHED_PID,
            port: PORT,
            signal_port: &self.signals,
            clock: &self.clock,
            files: &self.files,
            tail: None,
            stdout,
            stderr,
        }
    }
}

#[test]
fn teardown_lock_runs_the_cleanup_once() {
    let world = teardown_world();
    let guard = AtomicBool::new(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let first = run_teardown(world.request(&mut stdout, &mut stderr), &guard);
    let second = run_teardown(world.request(&mut stdout, &mut stderr), &guard);
    assert_eq!((first, second), (true, false));
    assert_eq!(world.signals.sent(), [(LAUNCHED_PID, TermSignal::Term)]);
    let text = String::from_utf8(stdout).unwrap();
    assert_eq!(text.matches("Stopped ephemeral proxy").count(), 1);
    assert!(stderr.is_empty());
}

#[test]
fn teardown_restores_the_original_settings_and_clears_the_record() {
    let world = teardown_world();
    let guard = AtomicBool::new(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert!(run_teardown(
        world.request(&mut stdout, &mut stderr),
        &guard
    ));
    assert_eq!(model_field(&world.settings).as_deref(), Some("old"));
    assert!(!world.backup.exists());
    assert!(!world.pid_record.exists());
}

#[test]
fn stop_tolerates_corrupt_pid_and_still_restores_settings() {
    let mut fixture = default_fixture();
    let settings = fixture.settings_path();
    let backup_path = fixture.backup_path();
    let pid_record_path = fixture.pid_record_path();
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"model":"patched"}"#).unwrap();
    let original: SettingsDocument = serde_json::from_str(r#"{"model":"original"}"#).unwrap();
    write_backup(
        &BackupRecord {
            existed: true,
            content: Some(original),
        },
        &backup_path,
        &fixture.files,
    )
    .unwrap();
    fs::write(&pid_record_path, "{not json").unwrap();
    fixture.stop().unwrap();
    let stderr = fixture.stderr_text();
    assert!(stderr.contains("invalid or unexpected JSON"), "{stderr}");
    assert!(
        stderr.contains("Clearing it and continuing cleanup."),
        "{stderr}"
    );
    assert!(!pid_record_path.exists());
    assert_eq!(model_field(&settings).as_deref(), Some("original"));
    assert!(!backup_path.exists());
    assert!(fixture.stdout_text().contains("Restored"));
}

#[test]
fn stop_terminates_a_leftover_proxy_and_clears_the_record() {
    let mut fixture = default_fixture();
    write_pid_record(&fixture.pid_record_path(), &record(777)).unwrap();
    fixture.signals.alive.lock().unwrap().push(777);
    fixture.stop().unwrap();
    assert_eq!(fixture.signals.sent(), [(777, TermSignal::Term)]);
    assert!(!fixture.pid_record_path().exists());
    let stdout = fixture.stdout_text();
    assert!(
        stdout.contains("Stopped leftover ephemeral proxy (pid 777)."),
        "{stdout}"
    );
    assert!(stdout.contains("Nothing to restore."), "{stdout}");
}

#[test]
fn deprecated_alias_up_warns_and_starts_with_the_same_port() {
    let mut fixture = default_fixture();
    fixture
        .run(AutorouteCommand::Up(StartArgs { port: 3636 }))
        .unwrap();
    assert_eq!(
        fixture.stderr_text(),
        format!("{}\n", deprecation_notice("up", "start"))
    );
    let recorded = fixture.launcher.requests.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0]
            .arguments
            .windows(2)
            .any(|pair| pair == ["--port", "3636"])
    );
}

#[test]
fn deprecated_alias_down_warns_and_stops() {
    let mut fixture = default_fixture();
    fixture.run(AutorouteCommand::Down).unwrap();
    assert_eq!(
        fixture.stderr_text(),
        format!("{}\n", deprecation_notice("down", "stop"))
    );
    assert!(fixture.stdout_text().contains("Nothing to restore."));
}

struct RecordingHttp {
    statuses: Mutex<Vec<u16>>,
    urls: Mutex<Vec<String>>,
    timeouts: Mutex<Vec<u64>>,
    fail: bool,
}

impl Http for RecordingHttp {
    fn get(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        if let Ok(mut urls) = self.urls.lock() {
            urls.push(url.to_owned());
        }
        if let Ok(mut timeouts) = self.timeouts.lock() {
            timeouts.push(timeout_seconds);
        }
        if self.fail {
            return Err(PkceFailure::Protocol("connection refused".to_owned()));
        }
        let status = match self.statuses.lock() {
            Ok(mut statuses) => statuses.pop().unwrap_or(200),
            Err(_poisoned) => 200,
        };
        Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    fn post(
        &self,
        _url: &str,
        _body: HttpBody,
        _timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        Err(PkceFailure::Protocol(
            "the liveliness probe never posts".to_owned(),
        ))
    }
}

fn recording_http(statuses: Vec<u16>, fail: bool) -> RecordingHttp {
    RecordingHttp {
        statuses: Mutex::new(statuses),
        urls: Mutex::new(Vec::new()),
        timeouts: Mutex::new(Vec::new()),
        fail,
    }
}

#[test]
fn http_liveliness_asks_the_liveliness_url_with_a_two_second_timeout() {
    let http = recording_http(Vec::new(), false);
    let probe = HttpLiveliness { http: &http };
    assert!(probe.is_live("http://127.0.0.1:5483/health/liveliness"));
    assert_eq!(
        *http.urls.lock().unwrap(),
        ["http://127.0.0.1:5483/health/liveliness"]
    );
    assert_eq!(*http.timeouts.lock().unwrap(), [LIVELINESS_TIMEOUT_SECONDS]);
}

#[test]
fn http_liveliness_reports_false_for_a_non_200_answer() {
    let http = recording_http(vec![503], false);
    let probe = HttpLiveliness { http: &http };
    assert!(!probe.is_live("http://127.0.0.1:5483/health/liveliness"));
}

#[test]
fn http_liveliness_reports_false_when_the_request_fails() {
    let http = recording_http(Vec::new(), true);
    let probe = HttpLiveliness { http: &http };
    assert!(!probe.is_live("http://127.0.0.1:5483/health/liveliness"));
}

#[test]
fn configure_runs_the_wizard_with_the_cli_context() {
    let mut fixture = default_fixture();
    fixture.client.models = vec![chat_model("gpt-4o")];
    fixture.prompter.selections = VecDeque::from(vec![vec!["gpt-4o".to_owned()]; 4]);
    fixture.prompter.confirm_answers.push_back(false);
    fixture.prompter.confirm_answers.push_back(false);
    fixture.run(AutorouteCommand::Configure).unwrap();
    let config = fs::read_to_string(fixture.config_path()).unwrap();
    assert!(config.contains("litellm_proxy/gpt-4o"), "{config}");
    assert!(config.contains("http://127.0.0.1:4000"), "{config}");
    assert!(config.contains(support::CLI_API_KEY), "{config}");
}

#[test]
fn configure_refuses_a_non_interactive_terminal() {
    let mut fixture = default_fixture();
    fixture.stdin_is_terminal = false;
    fixture.client.models = vec![chat_model("gpt-4o")];
    let error = fixture.run(AutorouteCommand::Configure).unwrap_err();
    assert!(matches!(
        error,
        CommandsError::Wizard(WizardError::NotInteractive)
    ));
}
