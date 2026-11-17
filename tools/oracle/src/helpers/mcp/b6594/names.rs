use std::collections::BTreeSet;

use betterlite_mcp::server_utils::names::{
    add_server_prefix_to_name, is_tool_name_prefixed, known_tool_name_spellings,
    match_known_tool_name, openapi_tool_name, short_tool_prefix_enabled,
};
use betterlite_mcp::server_utils::prefixes::{
    PrefixPolicy, iter_known_server_prefixes, match_known_server_prefix,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::{Value, json};

use super::specs::{ServerSpec, server_from};

const SEPARATOR: &str = "-";

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NamesInput {
    AddServerPrefixToName(PrefixNameInput),
    ShortToolPrefixEnabled(ShortFlagInput),
    KnownToolNameSpellings(SpellingsInput),
    MatchKnownToolName(MatchToolInput),
    MatchKnownServerPrefix(MatchPrefixInput),
    IsToolNamePrefixed(PrefixedInput),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrefixNameInput {
    pub name: String,
    pub server_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShortFlagInput {
    #[serde(default)]
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpellingsInput {
    pub tool_name: String,
    pub server: ServerSpec,
    #[serde(default)]
    pub short_prefix_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchToolInput {
    pub tool_name: String,
    pub server: ServerSpec,
    pub names: Vec<String>,
    #[serde(default)]
    pub short_prefix_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchPrefixInput {
    pub name: String,
    pub known_prefixes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrefixedInput {
    pub tool_name: String,
    #[serde(default)]
    pub known_prefixes: Option<Vec<String>>,
}

pub(crate) fn run(input: &NamesInput) -> HelperResult {
    match input {
        NamesInput::AddServerPrefixToName(fields) => Ok(json!(add_server_prefix_to_name(
            &fields.name,
            &fields.server_name,
            SEPARATOR
        ))),
        NamesInput::ShortToolPrefixEnabled(fields) => {
            Ok(json!(short_tool_prefix_enabled(fields.raw.as_deref())))
        }
        NamesInput::KnownToolNameSpellings(fields) => spellings(fields),
        NamesInput::MatchKnownToolName(fields) => match_tool(fields),
        NamesInput::MatchKnownServerPrefix(fields) => Ok(match_prefix(fields)),
        NamesInput::IsToolNamePrefixed(fields) => Ok(json!(prefixed(fields))),
    }
}

fn policy(short_prefix_enabled: bool) -> PrefixPolicy {
    PrefixPolicy {
        separator: SEPARATOR.to_owned(),
        short_prefix_enabled,
    }
}

fn spellings(fields: &SpellingsInput) -> Result<Value, Box<HelperError>> {
    let server = server_from(&fields.server)?;
    let prefixes = iter_known_server_prefixes(&server, &policy(fields.short_prefix_enabled));
    Ok(json!({
        "spellings": known_tool_name_spellings(&fields.tool_name, &prefixes, SEPARATOR),
        "openapi": openapi_tool_name(&fields.tool_name),
    }))
}

fn match_tool(fields: &MatchToolInput) -> Result<Value, Box<HelperError>> {
    let server = server_from(&fields.server)?;
    let prefixes = iter_known_server_prefixes(&server, &policy(fields.short_prefix_enabled));
    let is_openapi = server.fields.spec_path.is_some();
    let names: Vec<&str> = fields.names.iter().map(String::as_str).collect();
    Ok(json!(match_known_tool_name(
        &fields.tool_name,
        is_openapi,
        &prefixes,
        SEPARATOR,
        names
    )))
}

fn match_prefix(fields: &MatchPrefixInput) -> Value {
    json!(match_known_server_prefix(
        &fields.name,
        &fields.known_prefixes,
        &policy(false)
    ))
}

fn prefixed(fields: &PrefixedInput) -> bool {
    let known: Option<BTreeSet<String>> = fields
        .known_prefixes
        .as_ref()
        .map(|prefixes| prefixes.iter().cloned().collect());
    is_tool_name_prefixed(&fields.tool_name, known.as_ref(), SEPARATOR)
}
