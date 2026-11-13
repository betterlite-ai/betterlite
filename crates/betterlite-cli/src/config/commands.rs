use std::io::Write;

use betterlite_config::loader::EnvironmentLookup;
use clap::Subcommand;

use super::error::ConfigError;
use super::{ConfigKey, load_config, save_config};

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    #[command(
        about = "Set a config KEY to VALUE (e.g. `lite config set base_url https://your-proxy.example.com`)"
    )]
    Set { key: String, value: String },
    #[command(about = "Print the value of KEY, or all stored config when KEY is omitted")]
    Get { key: Option<String> },
    #[command(about = "Remove KEY from the config file")]
    Unset { key: String },
}

pub fn run_config_command(
    command: ConfigCommand,
    environment: &dyn EnvironmentLookup,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), ConfigError> {
    match command {
        ConfigCommand::Set { key, value } => set_config(&key, &value, environment, stdout, stderr),
        ConfigCommand::Get { key } => get_config(key.as_deref(), environment, stdout, stderr),
        ConfigCommand::Unset { key } => unset_config(&key, environment, stdout, stderr),
    }
}

fn set_config(
    key: &str,
    value: &str,
    environment: &dyn EnvironmentLookup,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), ConfigError> {
    let config_key = ConfigKey::from_name(key).ok_or_else(|| {
        ConfigError::Usage(format!(
            "Unknown config key '{key}'. Allowed keys: base_url, hidden_commands"
        ))
    })?;
    let normalized = config_key.normalize(value)?;
    let mut config = load_config(environment, stderr);
    let name = config_key.name();
    config.insert(name.to_owned(), normalized.clone());
    let path = save_config(&config, environment)?;
    // a closed stdout loses the line, not the value that already reached the file
    let _unused = writeln!(stdout, "Set {name} = {normalized} in {}", path.display());
    Ok(())
}

fn get_config(
    key: Option<&str>,
    environment: &dyn EnvironmentLookup,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), ConfigError> {
    let config = load_config(environment, stderr);
    if let Some(name) = key {
        let stored = config
            .get(name)
            .ok_or_else(|| ConfigError::KeyNotSet(name.to_owned()))?;
        let _unused = writeln!(stdout, "{stored}");
        return Ok(());
    }
    if config.is_empty() {
        let _unused = writeln!(stdout, "(no config set)");
        return Ok(());
    }
    for (name, value) in &config {
        let _unused = writeln!(stdout, "{name} = {value}");
    }
    Ok(())
}

fn unset_config(
    key: &str,
    environment: &dyn EnvironmentLookup,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), ConfigError> {
    let mut config = load_config(environment, stderr);
    if config.remove(key).is_none() {
        let _unused = writeln!(stdout, "{key} was not set");
        return Ok(());
    }
    let path = save_config(&config, environment)?;
    let _unused = writeln!(stdout, "Removed {key} from {}", path.display());
    Ok(())
}
