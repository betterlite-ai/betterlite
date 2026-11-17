use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::{BTreeMap, BTreeSet};
mod tests;
const PREFIX_SEPARATOR_VAR: &str = "MCP_TOOL_PREFIX_SEPARATOR";
const STDIO_ENABLED_VAR: &str = "LITELLM_ENABLE_MCP_STDIO";
const STDIO_EXTRA_COMMANDS_VAR: &str = "LITELLM_MCP_STDIO_EXTRA_COMMANDS";
const DEFAULT_PREFIX_SEPARATOR: &str = "-";
// litellm/constants.py's MCP_STDIO_ALLOWED_COMMANDS base set
const BASE_ALLOWED_COMMANDS: [&str; 7] =
    ["npx", "uvx", "python", "python3", "node", "docker", "deno"];
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct Raw(pub BTreeMap<String, Box<IgnoredAny>>);
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct StdioPolicy {
    pub enabled: bool,
    pub allowed_commands: BTreeSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadRules {
    pub prefix_separator: String,
    pub stdio: StdioPolicy,
}
pub type Settings = PayloadRules;
impl Default for PayloadRules {
    fn default() -> Self {
        Self {
            prefix_separator: DEFAULT_PREFIX_SEPARATOR.to_owned(),
            stdio: StdioPolicy {
                enabled: false,
                allowed_commands: default_allowed_commands(),
            },
        }
    }
}
impl Section for Raw {
    type Settings = PayloadRules;
    fn convert<L>(self, lookup: &L) -> Result<PayloadRules, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix_separator = lookup
            .lookup(PREFIX_SEPARATOR_VAR)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_PREFIX_SEPARATOR.to_owned());
        let flag_text = lookup.lookup(STDIO_ENABLED_VAR).unwrap_or_default();
        let enabled = betterlite_env::flag(STDIO_ENABLED_VAR, &flag_text)?.unwrap_or(false);
        Ok(PayloadRules {
            prefix_separator,
            stdio: StdioPolicy {
                enabled,
                allowed_commands: allowed_commands(
                    lookup.lookup(STDIO_EXTRA_COMMANDS_VAR).as_deref(),
                ),
            },
        })
    }
}
// constants.py: the base set plus the comma parts of the extra list, empty parts dropped
fn allowed_commands(extra: Option<&str>) -> BTreeSet<String> {
    let mut commands = default_allowed_commands();
    let Some(list) = extra else {
        return commands;
    };
    commands.extend(
        list.split(',')
            .filter(|command| !command.is_empty())
            .map(str::to_owned),
    );
    commands
}
fn default_allowed_commands() -> BTreeSet<String> {
    BASE_ALLOWED_COMMANDS
        .into_iter()
        .map(str::to_owned)
        .collect()
}
