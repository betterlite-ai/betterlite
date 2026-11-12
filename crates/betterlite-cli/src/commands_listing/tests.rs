#![cfg(test)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use betterlite_config::loader::EnvironmentLookup;

use crate::agent_dispatch::agent_commands;
use crate::commands_listing::{STATIC_COMMANDS, TRAILING_COMMANDS, show_commands};

struct HomeEnv {
    home: Option<PathBuf>,
}

impl EnvironmentLookup for HomeEnv {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

fn listing(environment: &dyn EnvironmentLookup) -> String {
    let mut buffer = Vec::new();
    show_commands(&mut buffer, environment).expect("lists the commands");
    String::from_utf8(buffer).expect("the listing is UTF-8")
}

fn padded(commands: &[(&str, &str)]) -> String {
    commands
        .iter()
        .fold(String::new(), |mut text, (name, description)| {
            writeln!(text, "  {name:<20} {description}").expect("writes the listing");
            text
        })
}

fn agent_block() -> String {
    agent_commands()
        .iter()
        .fold(String::new(), |mut text, command| {
            let description = command
                .get_about()
                .map(ToString::to_string)
                .unwrap_or_default();
            writeln!(text, "  {:<20} {description}", command.get_name())
                .expect("writes the listing");
            text
        })
}

#[test]
fn show_commands_lists_the_static_commands_with_litellm_descriptions() {
    let text = listing(&HomeEnv { home: None });
    assert!(text.starts_with("Available commands:\n"));
    assert!(text.contains(&padded(&STATIC_COMMANDS)));
    assert!(text.contains(&format!(
        "  {:<20} {}\n",
        "login", "Authenticate with the LiteLLM proxy server"
    )));
    assert!(text.contains(&format!("  {:<20} {}\n", "users", "Manage users")));
}

#[test]
fn show_commands_appends_the_agent_commands_before_version_help_and_quit() {
    let text = listing(&HomeEnv { home: None });
    let agents = agent_block();
    let trailing = padded(&TRAILING_COMMANDS);
    let agent_at = text.find(&agents).expect("the agent commands");
    let trailing_at = text.find(&trailing).expect("version, help and quit");
    assert!(agent_at < trailing_at);
    assert!(agents.contains(&format!(
        "  {:<20} Run Claude Code through your LiteLLM proxy\n",
        "claude"
    )));
}

#[test]
fn show_commands_filters_hidden_command_names_from_the_stored_config() {
    let home = tempfile::tempdir().expect("a temporary home");
    let config_directory = home.path().join(".litellm");
    fs::create_dir_all(&config_directory).expect("creates the config directory");
    fs::write(
        config_directory.join("config.json"),
        r#"{"hidden_commands":"codex,login"}"#,
    )
    .expect("writes the hidden commands");
    let text = listing(&HomeEnv {
        home: Some(home.path().to_path_buf()),
    });
    let names: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .map(|rest| rest.split_whitespace().next().unwrap_or_default())
        .collect();
    assert!(!names.contains(&"login"));
    assert!(!names.contains(&"codex"));
    assert!(names.contains(&"logout"));
    assert!(names.contains(&"claude"));
    assert!(names.contains(&"version"));
}

#[test]
fn show_commands_pads_names_to_twenty_columns_and_ends_with_a_blank_line() {
    let text = listing(&HomeEnv { home: None });
    assert!(text.ends_with("\n\n"));
    assert!(text.contains(&format!(
        "  {:<20} {}\n",
        "chat", "Interactive streaming chat with models"
    )));
    assert!(text.contains(&format!(
        "  {:<20} {}\n",
        "pi", "Run pi through your LiteLLM proxy"
    )));
}
