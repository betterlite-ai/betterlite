#![cfg(test)]

mod support;

use secrecy::ExposeSecret as _;

use clap::Command;

use crate::agent_dispatch::error::AgentDispatchError;
use crate::agent_dispatch::{AgentCommandArgs, agent_commands, parse_agent_args};
use crate::agent_launch::KNOWN_AGENTS;
use crate::token_records::record::without_secret;
use crate::token_records::secret::encode_secret;

use support::MemoryFiles;

#[test]
fn resolve_api_key_returns_the_explicit_key_without_touching_storage() {
    let mut world = support::world(Vec::new());
    world.context.api_key = Some(support::secret("sk-explicit"));
    let (result, out) = support::resolve(&mut world);
    assert_eq!(
        result.expect("the explicit key").expose_secret(),
        "sk-explicit"
    );
    assert!(world.http.requests().is_empty());
    assert_eq!(out, "");
}

#[test]
fn resolve_api_key_returns_the_stored_key_for_the_context_base_url() {
    let full = support::stored_record(Some(support::secret("sk-stored")));
    let mut world = support::world(Vec::new());
    world.files = MemoryFiles::holding(without_secret(&full));
    world.vault.blob = Some(encode_secret(&full).expect("encodes the fixture"));
    let (result, _out) = support::resolve(&mut world);
    assert_eq!(result.expect("the stored key").expose_secret(), "sk-stored");
    assert!(world.http.requests().is_empty());
}

#[test]
fn resolve_api_key_errors_without_a_terminal_and_without_a_key() {
    let mut world = support::world(Vec::new());
    let (result, out) = support::resolve(&mut world);
    let error = result.expect_err("no key is available");
    assert!(matches!(&error, AgentDispatchError::NoKey));
    assert_eq!(
        error.to_string(),
        "No LiteLLM key found. Set LITELLM_PROXY_API_KEY (or pass --api-key) for non-interactive use, or run `lite login` from a terminal."
    );
    assert_eq!(out, "");
    assert!(world.http.requests().is_empty());
}

#[test]
fn resolve_api_key_logs_in_when_interactive_and_nothing_is_stored() {
    let mut world = support::world(support::sso_routes());
    world.terminal.interactive = true;
    let (result, out) = support::resolve(&mut world);
    assert_eq!(
        result.expect("the key the login stored").expose_secret(),
        "sk-sso"
    );
    assert!(out.starts_with("No LiteLLM credentials found; starting login...\n"));
    assert!(out.contains("Login successful!"));
    assert!(world.files.stored().is_some());
}

#[test]
fn resolve_api_key_errors_when_the_login_cannot_store_the_key() {
    let mut world = support::world(support::sso_routes());
    world.terminal.interactive = true;
    world.files.refuse = true;
    let (result, out) = support::resolve(&mut world);
    let error = result.expect_err("the login stored nothing");
    assert!(matches!(&error, AgentDispatchError::LoginProducedNoKey));
    assert_eq!(
        error.to_string(),
        "Login did not produce an API key; cannot start the agent."
    );
    assert!(out.contains("Signed in, but the credential could not be saved"));
}

#[test]
fn agent_commands_build_one_command_per_known_agent() {
    let commands = agent_commands();
    let names: Vec<&str> = commands.iter().map(Command::get_name).collect();
    let expected: Vec<&str> = KNOWN_AGENTS
        .iter()
        .map(|(binary, _name, _profiles, _hidden)| *binary)
        .collect();
    assert_eq!(names, expected);
    let short_helps: Vec<String> = commands
        .iter()
        .map(|command| {
            command
                .get_about()
                .map(ToString::to_string)
                .unwrap_or_default()
        })
        .collect();
    let expected_helps: Vec<String> = KNOWN_AGENTS
        .iter()
        .map(|(_binary, name, _profiles, _hidden)| format!("Run {name} through your LiteLLM proxy"))
        .collect();
    assert_eq!(short_helps, expected_helps);
    let long_helps: Vec<bool> = commands
        .iter()
        .map(|command| command.get_long_about().is_some())
        .collect();
    assert_eq!(long_helps, vec![true, true, true, true]);
}

#[test]
fn agent_commands_hide_the_agents_the_table_marks_hidden() {
    let hidden: Vec<bool> = agent_commands().iter().map(Command::is_hide_set).collect();
    assert_eq!(hidden, vec![false, false, false, true]);
}

fn tail(tokens: &[&str]) -> Vec<String> {
    tokens.iter().map(|token| String::from(*token)).collect()
}

#[test]
fn agent_commands_mount_skip_verify_and_forwarded_arguments() {
    let commands = agent_commands();
    commands[0]
        .clone()
        .try_get_matches_from(["claude", "--skip-verify", "-p", "hi"])
        .expect("the command takes --skip-verify and a forwarded tail");
    let args = parse_agent_args(&tail(&["--skip-verify", "-p", "hi"]));
    assert!(args.skip_verify);
    assert_eq!(args.forwarded, vec![String::from("-p"), String::from("hi")]);
}

#[test]
fn agent_commands_forward_unknown_options_to_the_agent() {
    let commands = agent_commands();
    commands[0]
        .clone()
        .try_get_matches_from(["claude", "--resume", "abc"])
        .expect("the command forwards an unknown option");
    let args = parse_agent_args(&tail(&["--resume", "abc"]));
    assert!(!args.skip_verify);
    assert_eq!(
        args.forwarded,
        vec![String::from("--resume"), String::from("abc")]
    );
}

#[test]
fn agent_commands_consume_skip_verify_after_forwarded_values() {
    let args = parse_agent_args(&tail(&["-p", "hi", "--skip-verify"]));
    assert!(args.skip_verify);
    assert_eq!(args.forwarded, vec![String::from("-p"), String::from("hi")]);
}

#[test]
fn agent_commands_consume_skip_verify_after_an_unknown_option() {
    let args = parse_agent_args(&tail(&["--resume", "--skip-verify"]));
    assert!(args.skip_verify);
    assert_eq!(args.forwarded, vec![String::from("--resume")]);
}

#[test]
fn agent_commands_drop_the_terminator_and_forward_what_follows() {
    let args = parse_agent_args(&tail(&["x", "--", "y"]));
    assert!(!args.skip_verify);
    assert_eq!(args.forwarded, vec![String::from("x"), String::from("y")]);
}

#[test]
fn agent_commands_keep_skip_verify_after_the_terminator_as_a_value() {
    let args = parse_agent_args(&tail(&["x", "--", "--skip-verify"]));
    assert!(!args.skip_verify);
    assert_eq!(
        args.forwarded,
        vec![String::from("x"), String::from("--skip-verify")]
    );
}

#[test]
fn agent_commands_forward_skip_verify_after_a_leading_terminator() {
    let args = parse_agent_args(&tail(&["--", "--skip-verify"]));
    assert!(!args.skip_verify);
    assert_eq!(args.forwarded, vec![String::from("--skip-verify")]);
}

#[test]
fn agent_commands_keep_a_second_terminator_after_a_leading_one() {
    let args = parse_agent_args(&tail(&["--", "x", "--", "y"]));
    assert!(!args.skip_verify);
    assert_eq!(
        args.forwarded,
        vec![String::from("x"), String::from("--"), String::from("y")]
    );
}

#[test]
fn launch_resolves_the_key_prints_the_notice_and_runs_the_agent() {
    let mut world = support::world(Vec::new());
    world.context.api_key = Some(support::secret("sk-explicit"));
    let (result, out) = support::launch(
        &mut world,
        "claude",
        AgentCommandArgs {
            skip_verify: false,
            forwarded: vec![String::from("--resume")],
        },
    );
    assert_eq!(result.expect("launches the agent"), 0_i32);
    assert_eq!(
        out,
        "litellm: routing Claude Code through proxy at https://gateway.example\n"
    );
    let handoffs = world
        .launcher
        .handoffs
        .lock()
        .expect("the handoff log")
        .clone();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0].path, "/usr/bin/claude");
    assert_eq!(
        handoffs[0].args,
        vec![String::from("claude"), String::from("--resume")]
    );
    assert_eq!(
        handoffs[0]
            .env
            .get("ANTHROPIC_BASE_URL")
            .map(String::as_str),
        Some("https://gateway.example")
    );
    assert_eq!(
        *world.probe.calls.lock().expect("the probe log"),
        vec![format!("{}/v1/models", support::SERVER)]
    );
}

#[test]
fn launch_prints_the_routing_notice_before_the_key_is_verified() {
    let mut world = support::world(Vec::new());
    world.context.api_key = Some(support::secret("sk-explicit"));
    world.probe.status = 401;
    let (result, out) = support::launch(&mut world, "claude", AgentCommandArgs::default());
    let error = result.expect_err("rejects the key");
    assert!(matches!(&error, AgentDispatchError::Agent(_)));
    assert!(error.to_string().contains("LiteLLM rejected your key"));
    assert_eq!(
        out,
        "litellm: routing Claude Code through proxy at https://gateway.example\n"
    );
    assert!(
        world
            .launcher
            .handoffs
            .lock()
            .expect("the handoff log")
            .is_empty()
    );
}

#[test]
fn launch_skips_the_proxy_check_when_skip_verify_is_set() {
    let mut world = support::world(Vec::new());
    world.context.api_key = Some(support::secret("sk-explicit"));
    let (result, _out) = support::launch(
        &mut world,
        "claude",
        AgentCommandArgs {
            skip_verify: true,
            forwarded: Vec::new(),
        },
    );
    assert_eq!(result.expect("launches the agent"), 0_i32);
    assert!(world.probe.calls.lock().expect("the probe log").is_empty());
}

#[test]
fn launch_reports_a_missing_agent_binary_as_an_error() {
    let mut world = support::world(Vec::new());
    world.context.api_key = Some(support::secret("sk-explicit"));
    world.resolver.found = None;
    let (result, out) = support::launch(&mut world, "claude", AgentCommandArgs::default());
    let error = result.expect_err("reports the missing binary");
    assert!(matches!(&error, AgentDispatchError::Agent(_)));
    assert!(
        error
            .to_string()
            .starts_with("Could not find `claude` on your PATH.")
    );
    assert!(out.contains("litellm: routing Claude Code"));
    assert!(
        world
            .launcher
            .handoffs
            .lock()
            .expect("the handoff log")
            .is_empty()
    );
}
