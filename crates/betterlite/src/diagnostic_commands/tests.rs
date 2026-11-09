#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::PoisonError;

use betterlite_config::loader::ConfigLoader;
use betterlite_http::health_probe::{HealthTransport, ReqwestHealth, TransportError};
use rstest::rstest;

use super::{
    DiagnosticError, ProcessLauncher, SilentLauncher, config_validation_command, health_command,
    needs_ollama, ollama_serve, test_command, version_command,
};

struct CannedTransport {
    body: String,
    url: Mutex<Option<String>>,
}

impl CannedTransport {
    fn new(body: &str) -> Self {
        Self {
            body: body.to_owned(),
            url: Mutex::new(None),
        }
    }
}

impl HealthTransport for CannedTransport {
    fn get(&self, url: &str) -> Result<String, TransportError> {
        *self.url.lock().unwrap_or_else(PoisonError::into_inner) = Some(url.to_owned());
        Ok(self.body.clone())
    }
}

#[derive(Default)]
struct RecordingLauncher {
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl ProcessLauncher for RecordingLauncher {
    fn spawn_silent(&self, program: &str, args: &[&str]) -> Result<(), io::Error> {
        let recorded: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((program.to_owned(), recorded));
        Ok(())
    }
}

struct FailingLauncher;

impl ProcessLauncher for FailingLauncher {
    fn spawn_silent(&self, _program: &str, _args: &[&str]) -> Result<(), io::Error> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "ollama is not installed",
        ))
    }
}

#[test]
fn version_command_prints_the_given_version_between_blank_lines() {
    let mut writer = Vec::new();

    version_command("1.2.3", &mut writer).expect("writing succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(output, "\nLiteLLM: Current Version = 1.2.3\n\n");
}

#[test]
fn health_command_requests_the_health_path_and_prints_indented_json() {
    let transport = CannedTransport::new(r#"{"healthy_endpoints": [], "unhealthy_count": 0}"#);
    let mut writer = Vec::new();

    health_command("0.0.0.0", 4000, &transport, &mut writer).expect("the response is JSON");

    assert_eq!(
        *transport.url.lock().unwrap_or_else(PoisonError::into_inner),
        Some("http://0.0.0.0:4000/health".to_owned())
    );
    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(
        output,
        "\nLiteLLM: Health Testing models in config\n{\n    \"healthy_endpoints\": [],\n    \"unhealthy_count\": 0\n}\n"
    );
}

#[test]
fn health_command_rejects_a_body_that_is_not_json() {
    let transport = CannedTransport::new("not json");
    let mut writer = Vec::new();

    let error = health_command("127.0.0.1", 4000, &transport, &mut writer)
        .expect_err("the body is not JSON");

    assert!(matches!(error, DiagnosticError::NotJson { .. }));
}

#[test]
fn health_command_writes_the_status_line_before_a_transport_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let transport = ReqwestHealth::new().expect("the client builds");
    let mut writer = Vec::new();

    let error = health_command("127.0.0.1", port, &transport, &mut writer)
        .expect_err("nothing listens on the port");

    assert!(matches!(error, DiagnosticError::Transport { .. }));
    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(output, "\nLiteLLM: Health Testing models in config\n");
}

fn config_file(text: &str) -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("config.yaml");
    fs::write(&path, text).unwrap();
    (folder, path)
}

#[test]
fn config_validation_accepts_a_loadable_file() {
    let (_folder, path) = config_file(
        "model_list:\n  - model_name: gpt-4\n    litellm_params:\n      model: openai/gpt-4\n",
    );
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let mut writer = Vec::new();
    let mut err_writer = Vec::new();

    config_validation_command(Some(&path), &loader, &mut writer, &mut err_writer)
        .expect("the config loads");

    assert_eq!(
        String::from_utf8(writer).expect("writer output is valid utf8"),
        "LiteLLM: config OK (1 models)\n"
    );
    assert!(err_writer.is_empty());
}

#[test]
fn config_validation_requires_a_config_path() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let mut writer = Vec::new();
    let mut err_writer = Vec::new();

    let error = config_validation_command(None, &loader, &mut writer, &mut err_writer)
        .expect_err("no path is given");

    assert!(matches!(error, DiagnosticError::ConfigRequired));
    assert_eq!(
        error.to_string(),
        "--validate_config requires --config <path>"
    );
    assert!(writer.is_empty());
    assert!(err_writer.is_empty());
}

#[test]
fn config_validation_reports_a_file_that_does_not_load() {
    let (_folder, path) = config_file("model_list: [\n");
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let mut writer = Vec::new();
    let mut err_writer = Vec::new();

    let error = config_validation_command(Some(&path), &loader, &mut writer, &mut err_writer)
        .expect_err("the file does not load");

    assert!(matches!(error, DiagnosticError::ConfigInvalid { .. }));
    let reported = String::from_utf8(err_writer).expect("error output is valid utf8");
    assert!(
        reported.starts_with("LiteLLM: config validation failed: "),
        "{reported}"
    );
    assert!(writer.is_empty());
}

#[test]
fn command_prints_the_default_model_and_fails() {
    let mut writer = Vec::new();

    let error = test_command("0.0.0.0", 4000, None, &mut writer)
        .expect_err("the test value is always invalid");

    assert!(matches!(error, DiagnosticError::InvalidTestValue));
    assert_eq!(error.to_string(), "Invalid test value");
    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(
        output,
        "\nLiteLLM: Making a test ChatCompletions request to your proxy. Model=gpt-3.5-turbo\n"
    );
}

#[rstest]
#[case::named("my-model", "my-model")]
#[case::empty("", "gpt-3.5-turbo")]
fn command_uses_the_given_model_or_the_default(#[case] model: &str, #[case] expected: &str) {
    let mut writer = Vec::new();

    let error = test_command("0.0.0.0", 4000, Some(model), &mut writer)
        .expect_err("the test value is always invalid");

    assert!(matches!(error, DiagnosticError::InvalidTestValue));
    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(
        output,
        format!(
            "\nLiteLLM: Making a test ChatCompletions request to your proxy. Model={expected}\n"
        )
    );
}

#[rstest]
#[case::no_model(None, None, false)]
#[case::ollama_model(Some("ollama/llama3"), None, true)]
#[case::ollama_substring(Some("azure/my-ollama-proxy"), None, true)]
#[case::api_base_set(Some("ollama/llama3"), Some("http://localhost:11434"), false)]
#[case::empty_api_base(Some("ollama/llama3"), Some(""), false)]
#[case::other_model(Some("gpt-4"), None, false)]
fn needs_ollama_matches_only_a_model_without_api_base(
    #[case] model: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(needs_ollama(model, api_base), expected);
}

#[test]
fn ollama_serve_starts_ollama_serve_silently() {
    let launcher = RecordingLauncher::default();
    let mut writer = Vec::new();

    ollama_serve(&launcher, &mut writer).expect("the spawn succeeds");

    assert_eq!(
        *launcher
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        [("ollama".to_owned(), vec!["serve".to_owned()])]
    );
    assert!(writer.is_empty());
}

#[test]
fn ollama_serve_turns_a_spawn_failure_into_the_warning() {
    let mut writer = Vec::new();

    ollama_serve(&FailingLauncher, &mut writer).expect("a spawn failure only warns");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(
        output,
        "\n                LiteLLM Warning: proxy started with `ollama` model\n`ollama serve` failed with Exceptionollama is not installed. \nEnsure you run `ollama serve`\n            \n"
    );
}

#[test]
fn silent_launcher_starts_a_program_without_waiting() {
    SilentLauncher
        .spawn_silent("sh", &["-c", "exit 0"])
        .expect("sh starts");
}

#[test]
fn silent_launcher_reports_a_missing_program() {
    let error = SilentLauncher
        .spawn_silent("/nonexistent/ollama-diagnostic-probe", &[])
        .expect_err("the program does not exist");

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}
