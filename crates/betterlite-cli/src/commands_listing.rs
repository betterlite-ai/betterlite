use std::io;
use std::io::Write;

use betterlite_config::loader::EnvironmentLookup;
use clap::Command;

use crate::agent_dispatch::agent_commands;
use crate::config::hidden_command_names;

pub const AVAILABLE_COMMANDS: &str = "Available commands:";
pub const STATIC_COMMANDS: [(&str, &str); 10] = [
    ("login", "Authenticate with the LiteLLM proxy server"),
    ("logout", "Clear stored authentication"),
    ("whoami", "Show current authentication status"),
    ("models", "Manage and view model configurations"),
    ("credentials", "Manage API credentials"),
    ("chat", "Interactive streaming chat with models"),
    ("http", "Make HTTP requests to the proxy"),
    ("keys", "Manage API keys"),
    ("teams", "Manage teams and team assignments"),
    ("users", "Manage users"),
];
pub const TRAILING_COMMANDS: [(&str, &str); 3] = [
    ("version", "Show version information"),
    ("help", "Show this help message"),
    ("quit", "Exit the interactive session"),
];

#[must_use]
pub fn command_listing() -> Vec<(String, String)> {
    let mut commands: Vec<(String, String)> = STATIC_COMMANDS
        .iter()
        .map(|(name, description)| ((*name).to_owned(), (*description).to_owned()))
        .collect();
    commands.extend(agent_commands().iter().map(agent_entry));
    commands.extend(
        TRAILING_COMMANDS
            .iter()
            .map(|(name, description)| ((*name).to_owned(), (*description).to_owned())),
    );
    commands
}

fn agent_entry(command: &Command) -> (String, String) {
    let description = command
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    (command.get_name().to_owned(), description)
}

pub fn show_commands(
    stdout: &mut dyn Write,
    environment: &dyn EnvironmentLookup,
) -> io::Result<()> {
    let hidden = hidden_command_names(environment);
    writeln!(stdout, "{AVAILABLE_COMMANDS}")?;
    for (name, description) in command_listing() {
        if hidden.contains(&name) {
            continue;
        }
        writeln!(stdout, "  {name:<20} {description}")?;
    }
    writeln!(stdout)
}

mod tests;
