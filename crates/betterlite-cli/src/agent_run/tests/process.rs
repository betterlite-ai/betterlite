#[cfg(unix)]
fn shell_invocation(script: &str, env: EnvVars, timeout: Duration) -> DebugInvocation {
    DebugInvocation {
        command: WindowsCommand::Direct("/bin/sh".to_owned(), command(&["-c", script])),
        env,
        timeout,
    }
}

#[cfg(unix)]
#[test]
fn std_debug_models_returns_the_exit_code_and_both_streams() {
    let invocation = shell_invocation(
        "printf out; printf err >&2; exit 3",
        EnvVars::new(),
        Duration::from_secs(10_u64),
    );
    let output = StdDebugModels.run(&invocation).unwrap();
    assert_eq!(
        output,
        DebugOutput {
            code: 3_i32,
            stdout: "out".to_owned(),
            stderr: "err".to_owned(),
        }
    );
}

#[cfg(unix)]
#[test]
fn std_debug_models_runs_with_exactly_the_given_environment() {
    let invocation = shell_invocation(
        "printf '%s|%s' \"$BETTERLITE_PROBE\" \"${HOME-unset}\"",
        vars(&[("BETTERLITE_PROBE", "yes")]),
        Duration::from_secs(10_u64),
    );
    let output = StdDebugModels.run(&invocation).unwrap();
    assert_eq!(output.code, 0_i32);
    assert_eq!(output.stdout, "yes|unset");
}

#[cfg(unix)]
#[test]
fn std_debug_models_reports_a_killed_process_as_minus_one() {
    let invocation = shell_invocation("kill -9 $$", EnvVars::new(), Duration::from_secs(10_u64));
    assert_eq!(StdDebugModels.run(&invocation).unwrap().code, -1_i32);
}

#[cfg(unix)]
#[test]
fn std_debug_models_times_out_a_command_that_never_ends() {
    let invocation = shell_invocation(
        "while :; do :; done",
        EnvVars::new(),
        Duration::from_millis(200_u64),
    );
    let failure = StdDebugModels.run(&invocation).unwrap_err();
    assert!(matches!(failure, DebugFailure::Timeout));
}

#[cfg(unix)]
#[test]
fn std_debug_models_reports_output_that_is_not_utf8() {
    let invocation = shell_invocation(
        "printf '\\377'",
        EnvVars::new(),
        Duration::from_secs(10_u64),
    );
    let failure = StdDebugModels.run(&invocation).unwrap_err();
    assert!(matches!(failure, DebugFailure::Decode(_)));
}

#[cfg(unix)]
#[test]
fn std_debug_models_reports_a_program_that_does_not_exist() {
    let directory = temp_dir();
    let missing = directory.path().join("codex");
    let invocation = DebugInvocation {
        command: WindowsCommand::Direct(missing.to_str().unwrap().to_owned(), Vec::new()),
        env: EnvVars::new(),
        timeout: Duration::from_secs(10_u64),
    };
    let failure = StdDebugModels.run(&invocation).unwrap_err();
    assert!(
        matches!(failure, DebugFailure::Io(source) if source.kind() == io::ErrorKind::NotFound)
    );
}

#[cfg(unix)]
#[test]
fn std_debug_models_refuses_a_batch_command_off_windows() {
    let invocation = DebugInvocation {
        command: WindowsCommand::Batch("codex.cmd debug models".to_owned()),
        env: EnvVars::new(),
        timeout: Duration::from_secs(10_u64),
    };
    let failure = StdDebugModels.run(&invocation).unwrap_err();
    assert!(matches!(failure, DebugFailure::Io(_)));
}

#[test]
fn stream_messages_write_warnings_and_notices_to_their_own_streams() {
    let mut messages = StreamMessages {
        notices: Vec::new(),
        warnings: Vec::new(),
    };
    messages.warn("careful").unwrap();
    messages.notice("synced").unwrap();
    assert_eq!(messages.warnings, b"careful\n");
    assert_eq!(messages.notices, b"synced\n");
}

#[test]
fn std_statusline_installer_names_the_running_binary() {
    let command = StdStatuslineInstaller.install().unwrap();
    let expected = statusline_command(&env::current_exe().unwrap(), env::consts::OS);
    assert_eq!(command, expected.unwrap());
}
