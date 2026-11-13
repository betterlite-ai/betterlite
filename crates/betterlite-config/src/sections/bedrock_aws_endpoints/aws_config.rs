use std::collections::BTreeMap;
use std::fs;

use crate::loader::EnvironmentLookup;

mod ini;
mod path;

use ini::{Section, Value};

type Profile = BTreeMap<String, Value>;
type ProfileMap = BTreeMap<String, Profile>;

const CONFIG_VARIABLE: &str = "AWS_CONFIG_FILE";
const CONFIG_DEFAULT: &str = "~/.aws/config";
const CREDENTIALS_VARIABLE: &str = "AWS_SHARED_CREDENTIALS_FILE";
const CREDENTIALS_DEFAULT: &str = "~/.aws/credentials";
const DEFAULT_SECTION: &str = "DEFAULT";
const DEFAULT_PROFILE: &str = "default";

// boto3 returns a nested region as a dict, which LiteLLM's region validation rejects
pub(super) const NESTED_REGION: &str = "{nested}";

pub(crate) fn session_region<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    let config = read_sections(lookup, CONFIG_VARIABLE, CONFIG_DEFAULT)?;
    let credentials = read_sections(lookup, CREDENTIALS_VARIABLE, CREDENTIALS_DEFAULT)?;
    let profile = lookup
        .lookup("AWS_DEFAULT_PROFILE")
        .or_else(|| lookup.lookup("AWS_PROFILE"));
    let profiles = profile_map(&config, &credentials);
    if profile
        .as_deref()
        .is_some_and(|name| !profiles.contains_key(name))
    {
        return None;
    }
    let selected = profiles
        .get(profile.as_deref().unwrap_or(DEFAULT_PROFILE))
        .and_then(|values| values.get("region"))
        .map(|value| match value {
            Value::Scalar(text) => text.clone(),
            Value::Nested => NESTED_REGION.to_owned(),
        });
    lookup.lookup("AWS_DEFAULT_REGION").or(selected)
}

fn read_sections<L>(lookup: &L, variable: &str, fallback: &str) -> Option<Vec<Section>>
where
    L: EnvironmentLookup,
{
    let configured = lookup.lookup(variable);
    let path = path::expand(lookup, configured.as_deref().unwrap_or(fallback));
    if path.as_os_str().is_empty() {
        return Some(Vec::new());
    }
    let Ok(bytes) = fs::read(path) else {
        return Some(Vec::new());
    };
    let text = String::from_utf8(bytes).ok()?;
    ini::parse(&text).ok()
}

fn profile_map(config: &[Section], credentials: &[Section]) -> ProfileMap {
    let mut profiles = ProfileMap::new();
    for section in config {
        if let Some(name) = config_profile_name(&section.name) {
            profiles.insert(name, section.values.clone());
        }
    }
    overlay_credentials(&mut profiles, credentials);
    profiles
}

fn config_profile_name(name: &str) -> Option<String> {
    if name.starts_with("profile") {
        let words = split_name(name)?;
        let [_, profile] = words.as_slice() else {
            return None;
        };
        return Some(profile.clone());
    }
    if name == DEFAULT_PROFILE {
        return Some(DEFAULT_PROFILE.to_owned());
    }
    None
}

// Python shlex.split in posix mode without comments, which is what botocore's _parse_section calls
fn split_name(name: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    let mut chars = name.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(opening), closing) if opening == closing => quote = None,
            (Some('"'), '\\') => {
                let next = chars.next()?;
                if !matches!(next, '"' | '\\') {
                    word.get_or_insert_default().push('\\');
                }
                word.get_or_insert_default().push(next);
            }
            (None, '\'' | '"') => {
                quote = Some(ch);
                word.get_or_insert_default();
            }
            (None, '\\') => word.get_or_insert_default().push(chars.next()?),
            (None, ' ' | '\t' | '\r' | '\n') => {
                if let Some(done) = word.take() {
                    words.push(done);
                }
            }
            (_, other) => word.get_or_insert_default().push(other),
        }
    }
    if quote.is_some() {
        return None;
    }
    if let Some(done) = word {
        words.push(done);
    }
    Some(words)
}

fn overlay_credentials(profiles: &mut ProfileMap, sections: &[Section]) {
    for section in sections
        .iter()
        .filter(|section| section.name != DEFAULT_SECTION)
    {
        profiles
            .entry(section.name.clone())
            .or_default()
            .extend(section.values.clone());
    }
}
