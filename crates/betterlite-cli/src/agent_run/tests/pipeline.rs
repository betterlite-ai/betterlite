fn error_of(result: Result<i32, AgentRunError>) -> AgentRunError {
    let Err(error) = result else {
        panic!("expected an error");
    };
    error
}

#[test]
fn run_agent_rejects_an_empty_command() {
    let mut harness = Harness::codex();
    harness.command.clear();
    let (result, observations) = harness.run();
    assert!(matches!(error_of(result), AgentRunError::NothingToRun));
    assert!(observations.resolver_asked.is_empty());
    assert!(observations.launches.is_empty());
}

#[test]
fn run_agent_reports_a_missing_known_binary_with_install_docs() {
    let mut harness = Harness::codex();
    harness.binary = None;
    harness.command = command(&["codex"]);
    let (result, observations) = harness.run();
    let AgentRunError::MissingBinary { command, hint } = error_of(result) else {
        panic!("expected a missing binary error");
    };
    assert_eq!(command, "codex");
    assert_eq!(
        hint,
        " Install it first: https://developers.openai.com/codex/cli"
    );
    assert_eq!(observations.resolver_asked, ["codex"]);
    assert!(observations.probe_calls.is_empty());
}

#[test]
fn run_agent_reports_a_missing_unknown_binary_without_a_hint() {
    let mut harness = Harness::codex();
    harness.binary = None;
    harness.command = command(&["mytool"]);
    let (result, _observations) = harness.run();
    let AgentRunError::MissingBinary { command, hint } = error_of(result) else {
        panic!("expected a missing binary error");
    };
    assert_eq!(command, "mytool");
    assert!(hint.is_empty());
}

#[test]
fn run_agent_uses_the_basename_for_install_docs() {
    let mut harness = Harness::codex();
    harness.binary = None;
    harness.command = command(&["/opt/bin/codex"]);
    let (result, _observations) = harness.run();
    let AgentRunError::MissingBinary { command, hint } = error_of(result) else {
        panic!("expected a missing binary error");
    };
    assert_eq!(command, "/opt/bin/codex");
    assert_eq!(
        hint,
        " Install it first: https://developers.openai.com/codex/cli"
    );
}

#[test]
fn run_agent_verifies_the_proxy_key_before_launching() {
    let mut harness = Harness::codex();
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(observations.probe_calls.len(), 1_usize);
    assert_eq!(
        observations.probe_calls[0].url,
        "http://gateway.test:4000/v1/models"
    );
    assert!(observations.probe_calls[0].key_matches);
    assert_eq!(
        observations.probe_calls[0].timeout,
        Duration::from_secs(10_u64)
    );
    assert_eq!(observations.launches.len(), 1_usize);
}

#[test]
fn run_agent_skips_verification_when_flagged() {
    let mut harness = Harness::codex();
    harness.skip_verify = true;
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert!(observations.probe_calls.is_empty());
    assert!(observations.sync_calls[0].skip_verify);
}

#[test]
fn run_agent_reports_an_unreachable_proxy() {
    let mut harness = Harness::codex();
    harness.probe_failure = Some(io::ErrorKind::ConnectionRefused);
    let (result, observations) = harness.run();
    let AgentRunError::Launch(LaunchError::Unreachable(base, _source)) = error_of(result) else {
        panic!("expected an unreachable proxy error");
    };
    assert_eq!(base, "http://gateway.test:4000");
    assert!(observations.launches.is_empty());
    assert!(observations.sync_calls.is_empty());
}

#[test]
fn run_agent_reports_a_rejected_key() {
    let mut harness = Harness::codex();
    harness.probe_status = 403_u16;
    let (result, observations) = harness.run();
    let AgentRunError::Launch(LaunchError::RejectedKey(status)) = error_of(result) else {
        panic!("expected a rejected key error");
    };
    assert_eq!(status, 403_u16);
    assert!(observations.launches.is_empty());
}

#[test]
fn run_agent_warns_about_a_skipped_sync_and_still_launches() {
    let mut harness = Harness::codex();
    harness.sync_result = Some(ModelSyncResult::Skipped(ModelSyncSkipped {
        reason: "the proxy is offline".to_owned(),
    }));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(
        observations.warns,
        ["litellm: not syncing Codex models from the proxy: the proxy is offline"]
    );
    assert_eq!(observations.launches.len(), 1_usize);
}

#[test]
fn run_agent_places_sync_and_prepared_arguments_before_user_arguments() {
    let mut harness = Harness::codex();
    harness.command = command(&["codex", "--full-auto", "--model", "x"]);
    harness.sync_result = Some(ModelSyncResult::Args(ModelSyncArgs {
        args: command(&["-c", "model_catalog_json=\"catalog\""]),
    }));
    harness.preparer = PreparerScript::Returning(command(&["-c", "hooks.Stop=[]"]));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    let mut expected = command(&["codex"]);
    expected.extend(agent_launch_args("codex", BASE_URL));
    expected.extend(command(&["-c", "model_catalog_json=\"catalog\""]));
    expected.extend(command(&["-c", "hooks.Stop=[]"]));
    expected.extend(command(&["--full-auto", "--model", "x"]));
    assert_eq!(observations.launches[0].args, expected);
}

#[test]
fn run_agent_merges_the_sync_environment_over_the_agent_environment() {
    let mut harness = Harness::codex();
    harness.command = command(&["opencode"]);
    harness.binary = Some("/usr/local/bin/opencode".to_owned());
    harness.base_env = vars(&[("KEEP", "base")]);
    harness.sync_result = Some(ModelSyncResult::Env(vars(&[
        ("KEEP", "sync"),
        ("OPENCODE_CONFIG_CONTENT", "{\"provider\":{}}"),
    ])));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    let env = &observations.launches[0].env;
    assert_eq!(env.get("KEEP").map(String::as_str), Some("sync"));
    assert_eq!(
        env.get("OPENCODE_CONFIG_CONTENT").map(String::as_str),
        Some("{\"provider\":{}}")
    );
    assert_eq!(
        env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("http://gateway.test:4000/v1")
    );
    assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some(API_KEY));
}

#[test]
fn run_agent_uses_the_display_name_in_sync_warnings() {
    let mut harness = Harness::codex();
    harness.command = command(&["/opt/bin/mytool"]);
    harness.binary = Some("/opt/bin/mytool".to_owned());
    harness.sync_result = Some(ModelSyncResult::Skipped(ModelSyncSkipped {
        reason: "none".to_owned(),
    }));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(
        observations.warns,
        ["litellm: not syncing mytool models from the proxy: none"]
    );
}

#[test]
fn run_agent_calls_reattach_before_the_launch() {
    let mut harness = Harness::codex();
    harness.terminal = TerminalScript::Working;
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(observations.terminal_calls, 1_usize);
    assert_eq!(observations.terminal_calls_at_launch, Some(1_usize));
}

#[test]
fn run_agent_does_not_reattach_without_a_terminal() {
    let mut harness = Harness::codex();
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(observations.terminal_calls, 0_usize);
    assert_eq!(observations.terminal_calls_at_launch, Some(0_usize));
}

#[test]
fn run_agent_reports_a_terminal_failure_without_launching() {
    let mut harness = Harness::codex();
    harness.terminal = TerminalScript::Failing(io::ErrorKind::BrokenPipe);
    let (result, observations) = harness.run();
    let AgentRunError::Terminal { source } = error_of(result) else {
        panic!("expected a terminal error");
    };
    assert_eq!(source.kind(), io::ErrorKind::BrokenPipe);
    assert!(observations.launches.is_empty());
}

#[test]
fn run_agent_propagates_a_preparer_error() {
    let mut harness = Harness::codex();
    harness.preparer = PreparerScript::Failing;
    let (result, observations) = harness.run();
    assert!(matches!(error_of(result), AgentRunError::NothingToRun));
    assert!(observations.launches.is_empty());
}

#[test]
fn run_agent_propagates_a_sync_error() {
    let mut harness = Harness::codex();
    harness.sync_result = None;
    let (result, observations) = harness.run();
    assert!(matches!(error_of(result), AgentRunError::NothingToRun));
    assert!(observations.warns.is_empty());
    assert!(observations.launches.is_empty());
}

#[test]
fn run_agent_reports_a_launch_failure() {
    let mut harness = Harness::codex();
    harness.launcher_failure = Some(io::ErrorKind::PermissionDenied);
    let (result, observations) = harness.run();
    let AgentRunError::Launch(LaunchError::Process(source)) = error_of(result) else {
        panic!("expected a launch error");
    };
    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(observations.launches.len(), 1_usize);
}

#[test]
fn run_agent_returns_the_launcher_exit_code() {
    let mut harness = Harness::codex();
    harness.launcher_code = 7_i32;
    let (result, _observations) = harness.run();
    assert_eq!(result.unwrap(), 7_i32);
}

#[test]
fn run_agent_resolves_the_binary_for_the_sync_and_the_launch() {
    let mut harness = Harness::codex();
    harness.binary = Some("/opt/real-codex".to_owned());
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(observations.resolver_asked, ["codex"]);
    assert_eq!(observations.sync_calls[0].binary, "/opt/real-codex");
    assert_eq!(observations.launches[0].path, "/opt/real-codex");
    assert_eq!(observations.launches[0].args[0], "codex");
}

#[test]
fn run_agent_prepares_pi_even_when_verification_is_skipped() {
    let mut harness = Harness::codex();
    harness.command = command(&["pi"]);
    harness.binary = Some("/usr/bin/pi".to_owned());
    harness.skip_verify = true;
    harness.preparer = PreparerScript::Returning(command(&["--model", "litellm/gpt-4o"]));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert!(observations.probe_calls.is_empty());
    assert_eq!(observations.preparer_calls.len(), 1_usize);
    assert_eq!(
        observations.launches[0].args,
        command(&["pi", "--model", "litellm/gpt-4o"])
    );
}

#[test]
fn run_agent_does_not_prepare_a_command_that_keeps_its_extension() {
    let mut harness = Harness::codex();
    harness.command = command(&["codex.cmd", "--full-auto"]);
    harness.binary = Some("/opt/codex.cmd".to_owned());
    harness.preparer = PreparerScript::Returning(command(&["-c", "hooks.Stop=[]"]));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert!(observations.preparer_calls.is_empty());
    assert!(
        !observations.launches[0]
            .args
            .iter()
            .any(|argument| argument == "hooks.Stop=[]")
    );
}

#[test]
fn run_agent_selects_preparers_by_the_command_basename() {
    let directory = temp_dir();
    let environment = FakeEnvironment {
        variables: EnvVars::new(),
        home: Some(directory.path().to_path_buf()),
    };
    let installer = ScriptedInstaller::installing("/opt/betterlite statusline");
    let codex = CodexPreparer {
        install: &installer,
    };
    let http = ScriptedHttp::new(Vec::new());
    let files = ScriptedFiles::working();
    let pi = PiPreparer {
        get: &http,
        files: &files,
    };
    let preparers = Preparers {
        pi: &pi,
        codex: &codex,
    };
    let api_key = key();
    let request = PrepareRequest {
        base_url: BASE_URL,
        api_key: &api_key,
        environment: &environment,
    };
    let mut messages = RecordingMessages::default();
    let codex_args = preparers
        .get("codex")
        .unwrap()
        .prepare(&request, &mut messages)
        .unwrap();
    assert_eq!(codex_args.len(), 2_usize);
    assert_eq!(installer.calls(), 1_usize);
    assert!(preparers.get("pi").is_some());
    assert!(preparers.get("/opt/codex").is_none());
    assert!(preparers.get("codex.cmd").is_none());
}

#[test]
fn run_agent_prepares_a_command_given_by_path() {
    let mut harness = Harness::codex();
    harness.command = command(&["/opt/bin/codex", "--full-auto"]);
    harness.binary = Some("/opt/bin/codex".to_owned());
    harness.preparer = PreparerScript::Returning(command(&["-c", "hooks.Stop=[]"]));
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(observations.preparer_calls.len(), 1_usize);
    assert!(
        observations.launches[0]
            .args
            .contains(&"hooks.Stop=[]".to_owned())
    );
}

#[test]
fn run_agent_hands_the_batch_shim_to_the_windows_spawn() {
    let mut harness = Harness::codex();
    harness.command = command(&["codex.cmd", "--full-auto"]);
    harness.binary = Some("/opt/codex.cmd".to_owned());
    harness.platform = Platform::Windows;
    let (result, observations) = harness.run();
    assert_eq!(result.unwrap(), 0_i32);
    assert_eq!(observations.spawns.len(), 1_usize);
    assert!(matches!(observations.spawns[0].0, WindowsCommand::Batch(_)));
    assert!(observations.launches.is_empty());
}

#[test]
fn run_agent_hands_off_exactly_once() {
    let mut harness = Harness::codex();
    let (_result, observations) = harness.run();
    let handoffs = observations
        .launches
        .len()
        .saturating_add(observations.spawns.len());
    assert_eq!(handoffs, 1_usize);
    assert!(observations.notices.is_empty());
}
