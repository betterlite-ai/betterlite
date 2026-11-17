use std::collections::BTreeSet;

use crate::server_utils::prefixes::{PrefixPolicy, match_known_server_prefix};

mod tests;

#[must_use]
pub fn add_server_prefix_to_name(name: &str, server_name: &str, separator: &str) -> String {
    let mut prefixed = server_name.replace(' ', "_");
    prefixed.push_str(separator);
    prefixed.push_str(name);
    prefixed
}

#[must_use]
pub fn short_tool_prefix_enabled(raw: Option<&str>) -> bool {
    let Some(text) = raw else {
        return false;
    };
    // the one shared parser reads environment text into a flag: known spellings true, every other text false
    matches!(
        betterlite_env::flag("LITELLM_USE_SHORT_MCP_TOOL_PREFIX", text),
        Ok(Some(true))
    )
}

#[must_use]
pub fn known_tool_name_spellings(
    tool_name: &str,
    known_prefixes: &[String],
    separator: &str,
) -> Vec<String> {
    let mut spellings = Vec::with_capacity(known_prefixes.len().saturating_add(1));
    spellings.push(tool_name.to_owned());
    for prefix in known_prefixes {
        spellings.push(add_server_prefix_to_name(tool_name, prefix, separator));
    }
    spellings
}

#[must_use]
pub fn openapi_tool_name(operation_id: &str) -> String {
    let mut name = operation_id.replace(' ', "_");
    name = name.to_lowercase();
    name
}

#[must_use]
pub fn match_known_tool_name<'name, I>(
    tool_name: &str,
    is_openapi: bool,
    known_prefixes: &[String],
    separator: &str,
    names: I,
) -> Option<&'name str>
where
    I: IntoIterator<Item = &'name str>,
{
    let spellings: BTreeSet<String> =
        known_tool_name_spellings(tool_name, known_prefixes, separator)
            .into_iter()
            .map(|spelling| compare_form(&spelling, is_openapi))
            .collect();
    names
        .into_iter()
        .find(|name| spellings.contains(&compare_form(name, is_openapi)))
}

#[must_use]
pub fn is_tool_name_prefixed(
    tool_name: &str,
    known_server_prefixes: Option<&BTreeSet<String>>,
    separator: &str,
) -> bool {
    if !tool_name.contains(separator) {
        return false;
    }
    let Some(prefixes) = known_server_prefixes else {
        return true;
    };
    let policy = PrefixPolicy {
        separator: separator.to_owned(),
        short_prefix_enabled: false,
    };
    let candidates: Vec<String> = prefixes.iter().cloned().collect();
    match_known_server_prefix(tool_name, &candidates, &policy).is_some()
}

fn compare_form(name: &str, is_openapi: bool) -> String {
    if is_openapi {
        openapi_tool_name(name)
    } else {
        name.to_owned()
    }
}
