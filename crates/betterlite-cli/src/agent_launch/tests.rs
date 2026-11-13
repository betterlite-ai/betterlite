#![cfg(test)]

use std::fs::File;
use std::io::{self, ErrorKind, Write as _};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::{NamedTempFile, TempDir};

#[cfg(unix)]
use super::exec_command;
use super::{
    AgentRunError, EnvProfile, EnvVars, KNOWN_AGENTS, KeyProbe, LITELLM_PROXY_API_KEY_ENV,
    Platform, ProcessLauncher, Terminal, WindowsCommand, agent_launch_args, agent_profile,
    build_agent_env, codex_config_path, codex_declares_stop_hooks, codex_proxy_args,
    codex_proxy_provider, hand_off, verify_proxy_key, warn, windows_command,
};

fn vars(pairs: &[(&str, &str)]) -> EnvVars {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn value<'env>(env: &'env EnvVars, name: &str) -> Option<&'env str> {
    env.get(name).map(String::as_str)
}

fn key() -> SecretString {
    SecretString::from("sk-test")
}

struct Recorder {
    written: Vec<u8>,
    flushes: Vec<()>,
}

impl io::Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes.push(());
        Ok(())
    }
}

struct FakeEnvironment {
    variables: EnvVars,
    home: Option<PathBuf>,
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[derive(Clone, Debug)]
struct ProbeCall {
    url: String,
    key_matches: bool,
    timeout: Duration,
}

struct FakeProbe {
    calls: Mutex<Vec<ProbeCall>>,
    status: u16,
    failure: Option<ErrorKind>,
}

impl FakeProbe {
    fn returning(status: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            status,
            failure: None,
        }
    }

    fn failing(kind: ErrorKind) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            status: 0,
            failure: Some(kind),
        }
    }

    fn record(&self, call: ProbeCall) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<ProbeCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl KeyProbe for FakeProbe {
    fn status(&self, url: &str, api_key: &SecretString, timeout: Duration) -> io::Result<u16> {
        self.record(ProbeCall {
            url: url.to_owned(),
            key_matches: api_key.expose_secret() == "sk-test",
            timeout,
        });
        self.failure
            .map_or(Ok(self.status), |kind| Err(io::Error::from(kind)))
    }
}

struct FakeLauncher {
    replaces: Mutex<Vec<(String, Vec<String>, EnvVars)>>,
    spawns: Mutex<Vec<(WindowsCommand, EnvVars)>>,
    result: Result<i32, ErrorKind>,
}

impl FakeLauncher {
    fn exiting(code: i32) -> Self {
        Self {
            replaces: Mutex::new(Vec::new()),
            spawns: Mutex::new(Vec::new()),
            result: Ok(code),
        }
    }

    fn failing(kind: ErrorKind) -> Self {
        Self {
            replaces: Mutex::new(Vec::new()),
            spawns: Mutex::new(Vec::new()),
            result: Err(kind),
        }
    }

    fn result_value(&self) -> io::Result<i32> {
        match self.result {
            Ok(code) => Ok(code),
            Err(kind) => Err(io::Error::from(kind)),
        }
    }

    fn record_replace(&self, path: &str, args: &[String], env: &EnvVars) {
        self.replaces
            .lock()
            .unwrap()
            .push((path.to_owned(), args.to_vec(), env.clone()));
    }

    fn record_spawn(&self, command: &WindowsCommand, env: &EnvVars) {
        self.spawns
            .lock()
            .unwrap()
            .push((command.clone(), env.clone()));
    }
}

impl ProcessLauncher for FakeLauncher {
    fn replace(&self, path: &str, args: &[String], env: &EnvVars) -> io::Result<i32> {
        self.record_replace(path, args, env);
        self.result_value()
    }

    fn spawn(&self, command: &WindowsCommand, env: &EnvVars) -> io::Result<i32> {
        self.record_spawn(command, env);
        self.result_value()
    }
}

struct FakeTerminal {
    interactive: bool,
    controlling: Option<File>,
    opens: Mutex<Vec<()>>,
    attaches: Mutex<Vec<()>>,
    attach_failure: Option<ErrorKind>,
}

impl FakeTerminal {
    fn with_stdin(interactive: bool) -> Self {
        Self {
            interactive,
            controlling: None,
            opens: Mutex::new(Vec::new()),
            attaches: Mutex::new(Vec::new()),
            attach_failure: None,
        }
    }

    fn attached() -> Self {
        Self::with_stdin(true)
    }

    fn detached() -> Self {
        Self::with_stdin(false)
    }

    fn detached_with_file() -> Self {
        let mut terminal = Self::detached();
        terminal.controlling = Some(tempfile::tempfile().unwrap());
        terminal
    }

    fn record_open(&self) {
        self.opens.lock().unwrap().push(());
    }

    fn record_attach(&self) {
        self.attaches.lock().unwrap().push(());
    }

    fn open_count(&self) -> usize {
        self.opens.lock().unwrap().len()
    }

    fn attach_count(&self) -> usize {
        self.attaches.lock().unwrap().len()
    }
}

impl Terminal for FakeTerminal {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn open_controlling(&self) -> io::Result<File> {
        self.record_open();
        self.controlling.as_ref().map_or_else(
            || Err(io::Error::from(ErrorKind::NotFound)),
            File::try_clone,
        )
    }

    fn attach_stdin(&self, terminal: File) -> io::Result<()> {
        self.record_attach();
        drop(terminal);
        self.attach_failure
            .map_or(Ok(()), |kind| Err(io::Error::from(kind)))
    }
}

#[test]
fn agent_profile_known() {
    let (claude, claude_profiles) = agent_profile("/usr/local/bin/claude");
    assert_eq!(claude, "Claude Code");
    assert_eq!(claude_profiles, &[EnvProfile::Anthropic][..]);
    let (codex, codex_profiles) = agent_profile("codex");
    assert_eq!(codex, "Codex");
    assert_eq!(codex_profiles, &[EnvProfile::OpenAi][..]);
    let (opencode, opencode_profiles) = agent_profile("opencode");
    assert_eq!(opencode, "OpenCode");
    assert_eq!(opencode_profiles, &[EnvProfile::OpenAi][..]);
    let (pi, pi_profiles) = agent_profile("/usr/local/bin/pi");
    assert_eq!(pi, "pi");
    assert_eq!(pi_profiles, &[EnvProfile::ProxyKey][..]);
}

#[test]
fn known_agents_table_matches_litellm() {
    let commands = KNOWN_AGENTS.map(|(command, _, _, _)| command);
    assert_eq!(commands, ["claude", "codex", "opencode", "pi"]);
    let names = KNOWN_AGENTS.map(|(_, display_name, _, _)| display_name);
    assert_eq!(names, ["Claude Code", "Codex", "OpenCode", "pi"]);
    let hidden = KNOWN_AGENTS.map(|(_, _, _, hidden)| hidden);
    assert_eq!(hidden, [false, false, false, true]);
}

#[test]
fn agent_profile_unknown() {
    let (name, profiles) = agent_profile("/opt/tools/mytool");
    assert_eq!(name, "mytool");
    assert_eq!(profiles, &[EnvProfile::Anthropic, EnvProfile::OpenAi][..]);
}

#[test]
fn anthropic_env_routing() {
    let base = vars(&[("KEEP", "1")]);
    let env = build_agent_env(&base, "http://proxy///", &key(), &[EnvProfile::Anthropic]);
    assert_eq!(value(&env, "ANTHROPIC_BASE_URL"), Some("http://proxy"));
    assert_eq!(value(&env, "ANTHROPIC_AUTH_TOKEN"), Some("sk-test"));
    assert_eq!(value(&env, "KEEP"), Some("1"));
    assert_eq!(value(&env, "OPENAI_BASE_URL"), None);
    assert_eq!(value(&env, "OPENAI_API_KEY"), None);
    assert_eq!(value(&env, LITELLM_PROXY_API_KEY_ENV), None);
}

#[test]
fn anthropic_key_dropped() {
    let base = vars(&[("ANTHROPIC_API_KEY", "stray")]);
    let env = build_agent_env(&base, "http://proxy", &key(), &[EnvProfile::Anthropic]);
    assert_eq!(value(&env, "ANTHROPIC_API_KEY"), None);
    assert_eq!(value(&env, "ANTHROPIC_AUTH_TOKEN"), Some("sk-test"));
    let openai_env = build_agent_env(&base, "http://proxy", &key(), &[EnvProfile::OpenAi]);
    assert_eq!(value(&openai_env, "ANTHROPIC_API_KEY"), Some("stray"));
}

#[test]
fn anthropic_discovery_defaults() {
    let env = build_agent_env(
        &EnvVars::new(),
        "http://proxy",
        &key(),
        &[EnvProfile::Anthropic],
    );
    assert_eq!(value(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
    assert_eq!(
        value(&env, "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"),
        Some("1")
    );
}

#[test]
fn anthropic_discovery_preserved() {
    let base = vars(&[
        ("ENABLE_TOOL_SEARCH", ""),
        ("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY", "off"),
    ]);
    let env = build_agent_env(&base, "http://proxy", &key(), &[EnvProfile::Anthropic]);
    assert_eq!(value(&env, "ENABLE_TOOL_SEARCH"), Some(""));
    assert_eq!(
        value(&env, "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"),
        Some("off")
    );
}

#[test]
fn openai_env_routing() {
    let base = vars(&[("ANTHROPIC_API_KEY", "stray")]);
    let env = build_agent_env(&base, "http://proxy/", &key(), &[EnvProfile::OpenAi]);
    assert_eq!(value(&env, "OPENAI_BASE_URL"), Some("http://proxy/v1"));
    assert_eq!(value(&env, "OPENAI_API_KEY"), Some("sk-test"));
    assert_eq!(value(&env, "ANTHROPIC_BASE_URL"), None);
    assert_eq!(value(&env, "ENABLE_TOOL_SEARCH"), None);
    assert_eq!(value(&env, LITELLM_PROXY_API_KEY_ENV), None);
}

#[test]
fn litellm_env_routing() {
    let env = build_agent_env(
        &EnvVars::new(),
        "http://proxy",
        &key(),
        &[EnvProfile::ProxyKey],
    );
    assert_eq!(value(&env, LITELLM_PROXY_API_KEY_ENV), Some("sk-test"));
    assert_eq!(value(&env, "OPENAI_API_KEY"), None);
    assert_eq!(value(&env, "OPENAI_BASE_URL"), None);
    assert_eq!(value(&env, "ANTHROPIC_AUTH_TOKEN"), None);
}

#[test]
fn empty_profiles_keep_the_environment() {
    let base = vars(&[("KEEP", "1")]);
    let env = build_agent_env(&base, "http://proxy", &key(), &[]);
    assert_eq!(env, base);
}

#[test]
fn codex_config_path_default() {
    let environment = FakeEnvironment {
        variables: EnvVars::new(),
        home: Some(PathBuf::from("/home/tester")),
    };
    assert_eq!(
        codex_config_path(&environment),
        Some(PathBuf::from("/home/tester/.codex/config.toml"))
    );
}

#[test]
fn codex_config_path_home_override() {
    let environment = FakeEnvironment {
        variables: vars(&[("CODEX_HOME", "/opt/codex")]),
        home: Some(PathBuf::from("/home/tester")),
    };
    assert_eq!(
        codex_config_path(&environment),
        Some(PathBuf::from("/opt/codex/config.toml"))
    );
}

#[test]
fn codex_config_path_without_home_is_none() {
    let environment = FakeEnvironment {
        variables: EnvVars::new(),
        home: None,
    };
    assert_eq!(codex_config_path(&environment), None);
}

#[test]
fn codex_config_path_empty_home_falls_back_to_home() {
    let environment = FakeEnvironment {
        variables: vars(&[("CODEX_HOME", "")]),
        home: Some(PathBuf::from("/home/tester")),
    };
    assert_eq!(
        codex_config_path(&environment),
        Some(PathBuf::from("/home/tester/.codex/config.toml"))
    );
}

fn write_config(text: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file
}

#[rstest]
#[case::array_table("[[hooks.Stop]]\n")]
#[case::spaced_array_table("[[ \"hooks\" . \"Stop\" ]]\n")]
#[case::assignment("hooks.Stop = []\n")]
#[case::hooks_assignment("hooks = []\n")]
#[case::bare_table("[hooks]\n")]
#[case::leading_whitespace("  [[hooks.Stop]]\n")]
fn codex_stop_hooks_detected(#[case] text: &str) {
    let file = write_config(text);
    assert!(codex_declares_stop_hooks(file.path()));
}

#[rstest]
#[case::empty("")]
#[case::unrelated_table("[model]\nname = \"x\"\n")]
#[case::commented("# [[hooks.Stop]]\n")]
#[case::other_key("hook.Stop = []\n")]
#[case::similar_table("[hooks2]\n")]
#[case::inline_mention("name = \"[[hooks.Stop]]\"\n")]
fn codex_stop_hooks_absent(#[case] text: &str) {
    let file = write_config(text);
    assert!(!codex_declares_stop_hooks(file.path()));
}

#[test]
fn codex_stop_hooks_unreadable_config_returns_false() {
    let directory = TempDir::new().unwrap();
    assert!(!codex_declares_stop_hooks(
        &directory.path().join("missing.toml")
    ));
    assert!(!codex_declares_stop_hooks(directory.path()));
}

#[test]
fn codex_stop_hooks_invalid_utf8_returns_false() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&[0xff, 0xfe, 0x00]).unwrap();
    assert!(!codex_declares_stop_hooks(file.path()));
}

#[test]
fn codex_provider_frozen_mapping() {
    let provider = codex_proxy_provider("http://proxy///");
    assert_eq!(provider.name, "LiteLLM proxy");
    assert_eq!(provider.base_url, "http://proxy/v1");
    assert_eq!(provider.wire_api, "responses");
    assert!(!provider.supports_websockets);
    assert!(!provider.requires_openai_auth);
}

#[test]
fn codex_args_provider_override() {
    let args = agent_launch_args("/usr/local/bin/codex", "http://proxy/");
    let expected = [
        "-c",
        "model_provider=\"litellm\"",
        "-c",
        "model_providers.litellm.name=\"LiteLLM proxy\"",
        "-c",
        "model_providers.litellm.base_url=\"http://proxy/v1\"",
        "-c",
        "model_providers.litellm.wire_api=\"responses\"",
        "-c",
        "model_providers.litellm.supports_websockets=false",
        "-c",
        "model_providers.litellm.requires_openai_auth=false",
        "-c",
        "model_providers.litellm.env_key=\"OPENAI_API_KEY\"",
        "-c",
        "model_providers.litellm.http_headers={}",
    ];
    assert_eq!(args, expected.map(str::to_owned));
}

#[test]
fn codex_proxy_args_escapes_the_base_url() {
    let args = codex_proxy_args("http://proxy/a\"b\\c\nd");
    let expected = r#"model_providers.litellm.base_url="http://proxy/a\"b\\c\nd/v1""#;
    assert!(args.iter().any(|argument| argument.as_str() == expected));
}

#[test]
fn agent_launch_args_empty_for_other_agents() {
    assert_eq!(
        agent_launch_args("/usr/bin/claude", "http://proxy"),
        Vec::<String>::new()
    );
    assert_eq!(
        agent_launch_args("mytool", "http://proxy"),
        Vec::<String>::new()
    );
}

#[test]
fn proxy_401_rejected() {
    let probe = FakeProbe::returning(401);
    let error = verify_proxy_key("http://proxy/", &key(), &probe).unwrap_err();
    assert!(matches!(error, AgentRunError::RejectedKey(401)));
    let calls = probe.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, "http://proxy/v1/models");
    assert!(calls[0].key_matches);
    assert_eq!(calls[0].timeout, Duration::from_secs(10));
}

#[test]
fn proxy_403_rejected() {
    let probe = FakeProbe::returning(403);
    let error = verify_proxy_key("http://proxy", &key(), &probe).unwrap_err();
    assert_eq!(
        error.to_string(),
        "LiteLLM rejected your key (HTTP 403); run `lite login` to refresh it"
    );
    assert!(matches!(error, AgentRunError::RejectedKey(403)));
}

#[rstest]
#[case::ok(200)]
#[case::no_content(204)]
#[case::redirect(302)]
#[case::bad_request(400)]
#[case::not_found(404)]
#[case::server_error(500)]
#[case::unavailable(503)]
fn proxy_other_tolerated(#[case] status: u16) {
    let probe = FakeProbe::returning(status);
    assert!(matches!(
        verify_proxy_key("http://proxy", &key(), &probe),
        Ok(())
    ));
}

#[test]
fn proxy_unreachable_error() {
    let probe = FakeProbe::failing(ErrorKind::ConnectionRefused);
    let error = verify_proxy_key("http://proxy/", &key(), &probe).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("could not reach the LiteLLM proxy at http://proxy"));
    let AgentRunError::Unreachable(base, source) = error else {
        panic!("expected an unreachable error, got {error:?}");
    };
    assert_eq!(base, "http://proxy");
    assert_eq!(source.kind(), ErrorKind::ConnectionRefused);
}

#[rstest]
#[case::cmd("C:\\npm\\claude.cmd", &["claude", "--model", "a b"], r#"/d /e:on /v:off /s /c ""C:\npm\claude.cmd" "--model" "a b"""#)]
#[case::uppercase("C:\\npm\\CLAUDE.CMD", &["claude"], r#"/d /e:on /v:off /s /c ""C:\npm\CLAUDE.CMD"""#)]
#[case::bat("C:\\tools\\run.bat", &["run"], r#"/d /e:on /v:off /s /c ""C:\tools\run.bat"""#)]
fn windows_shim_routed_through_cmd(
    #[case] path: &str,
    #[case] args: &[&str],
    #[case] expected: &str,
) {
    let arguments = args
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<String>>();
    assert_eq!(
        windows_command(path, &arguments).unwrap(),
        WindowsCommand::Batch(expected.to_owned())
    );
}

#[test]
fn windows_non_shim_direct() {
    let args = vec!["codex".to_owned(), "--skip-verify".to_owned()];
    assert_eq!(
        windows_command("C:\\bin\\codex.exe", &args).unwrap(),
        WindowsCommand::Direct(
            "C:\\bin\\codex.exe".to_owned(),
            vec!["--skip-verify".to_owned()]
        )
    );
    assert_eq!(
        windows_command("/usr/bin/codex", &args).unwrap(),
        WindowsCommand::Direct(
            "/usr/bin/codex".to_owned(),
            vec!["--skip-verify".to_owned()]
        )
    );
}

#[rstest]
#[case::newline("a\nb")]
#[case::carriage_return("a\rb")]
#[case::lone_newline("\n")]
fn argument_with_linebreak_rejected(#[case] argument: &str) {
    let args = vec!["claude".to_owned(), argument.to_owned()];
    let error = windows_command("C:/npm/claude.cmd", &args).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot pass a line break to `claude.cmd` on Windows")
    );
    assert!(matches!(error, AgentRunError::LineBreak(agent) if agent == "claude.cmd"));
}

#[test]
fn linebreak_in_the_command_name_is_tolerated() {
    let args = vec!["a\nb".to_owned(), "--model".to_owned()];
    assert!(matches!(
        windows_command("C:/npm/claude.cmd", &args),
        Ok(WindowsCommand::Batch(_))
    ));
}

#[test]
fn linebreak_in_a_direct_argument_is_tolerated() {
    let args = vec!["codex".to_owned(), "a\nb".to_owned()];
    assert_eq!(
        windows_command("C:\\bin\\codex.exe", &args).unwrap(),
        WindowsCommand::Direct("C:\\bin\\codex.exe".to_owned(), vec!["a\nb".to_owned()])
    );
}

#[cfg(unix)]
#[test]
fn unix_exec_command_keeps_the_caller_name_as_argv_zero() {
    let env = vars(&[("KEEP", "1")]);
    let args = vec!["claude".to_owned(), "--model".to_owned()];
    let command = exec_command("/usr/bin/claude", &args, &env);
    assert_eq!(command.get_program().to_str(), Some("/usr/bin/claude"));
    let arguments = command
        .get_args()
        .map(|argument| argument.to_str().unwrap())
        .collect::<Vec<&str>>();
    assert_eq!(arguments, ["--model"]);
}

#[test]
fn unix_exec_process() {
    let launcher = FakeLauncher::exiting(0);
    let env = vars(&[("KEEP", "1")]);
    let args = vec!["claude".to_owned(), "--model".to_owned()];
    let code = hand_off("/usr/bin/claude", &args, &env, Platform::Unix, &launcher).unwrap();
    assert_eq!(code, 0_i32);
    assert!(launcher.spawns.lock().unwrap().is_empty());
    let replaces = launcher.replaces.lock().unwrap();
    let (path, replaced_args, replaced_env) = &replaces[0];
    assert_eq!(path, "/usr/bin/claude");
    assert_eq!(replaced_args, &args);
    assert_eq!(replaced_env, &env);
    drop(replaces);
}

#[test]
fn windows_spawn_child() {
    let launcher = FakeLauncher::exiting(0);
    let env = vars(&[("KEEP", "1")]);
    let args = vec!["claude".to_owned(), "--model".to_owned()];
    let code = hand_off(
        "C:\\npm\\claude.cmd",
        &args,
        &env,
        Platform::Windows,
        &launcher,
    )
    .unwrap();
    assert_eq!(code, 0_i32);
    assert!(launcher.replaces.lock().unwrap().is_empty());
    let spawns = launcher.spawns.lock().unwrap();
    let (command, spawn_env) = &spawns[0];
    assert_eq!(
        command,
        &WindowsCommand::Batch(
            r#"/d /e:on /v:off /s /c ""C:\npm\claude.cmd" "--model"""#.to_owned()
        )
    );
    assert_eq!(spawn_env, &env);
    drop(spawns);
}

#[test]
fn windows_exit_status_propagated() {
    let launcher = FakeLauncher::exiting(23);
    let args = vec!["codex".to_owned()];
    let code = hand_off(
        "C:\\bin\\codex.exe",
        &args,
        &EnvVars::new(),
        Platform::Windows,
        &launcher,
    )
    .unwrap();
    assert_eq!(code, 23_i32);
}

#[test]
fn hand_off_reports_a_failed_spawn() {
    let launcher = FakeLauncher::failing(ErrorKind::PermissionDenied);
    let args = vec!["codex".to_owned()];
    let error = hand_off(
        "C:\\bin\\codex.exe",
        &args,
        &EnvVars::new(),
        Platform::Windows,
        &launcher,
    )
    .unwrap_err();
    let AgentRunError::Process(source) = error else {
        panic!("expected a process error, got {error:?}");
    };
    assert_eq!(source.kind(), ErrorKind::PermissionDenied);
}

#[test]
fn hand_off_reports_a_failed_exec() {
    let launcher = FakeLauncher::failing(ErrorKind::NotFound);
    let args = vec!["claude".to_owned()];
    let error = hand_off(
        "/usr/bin/claude",
        &args,
        &EnvVars::new(),
        Platform::Unix,
        &launcher,
    )
    .unwrap_err();
    let AgentRunError::Process(source) = error else {
        panic!("expected a process error, got {error:?}");
    };
    assert_eq!(source.kind(), ErrorKind::NotFound);
}

#[test]
fn tty_undetached_and_attached() {
    let terminal = FakeTerminal::detached_with_file();
    terminal.restore_controlling_terminal().unwrap();
    assert_eq!(terminal.open_count(), 1);
    assert_eq!(terminal.attach_count(), 1);
}

#[test]
fn tty_already_attached_unchanged() {
    let terminal = FakeTerminal::attached();
    terminal.restore_controlling_terminal().unwrap();
    assert_eq!(terminal.open_count(), 0);
    assert_eq!(terminal.attach_count(), 0);
}

#[test]
fn tty_missing_controlling_terminal_is_tolerated() {
    let terminal = FakeTerminal::detached();
    terminal.restore_controlling_terminal().unwrap();
    assert_eq!(terminal.open_count(), 1);
    assert_eq!(terminal.attach_count(), 0);
}

#[test]
fn tty_attach_failure_is_an_error() {
    let mut terminal = FakeTerminal::detached_with_file();
    terminal.attach_failure = Some(ErrorKind::PermissionDenied);
    let error = terminal.restore_controlling_terminal().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
}

#[test]
fn interactive_true_on_tty() {
    assert!(FakeTerminal::attached().is_interactive());
}

#[test]
fn interactive_false_when_detached() {
    assert!(!FakeTerminal::detached().is_interactive());
}

#[test]
fn warn_writes_the_message_and_a_newline() {
    let mut output = Recorder {
        written: Vec::new(),
        flushes: Vec::new(),
    };
    warn(&mut output, "litellm: skipped model sync").unwrap();
    assert_eq!(output.written, b"litellm: skipped model sync\n");
    assert_eq!(output.flushes.len(), 1_usize);
}
