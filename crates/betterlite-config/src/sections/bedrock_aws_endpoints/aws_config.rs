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

enum Source {
    Missing,
    Text(String),
    Invalid,
}

impl Source {
    fn sections(&self) -> Option<Vec<Section>> {
        match self {
            Self::Missing => Some(Vec::new()),
            Self::Text(text) => ini::parse(text).ok(),
            Self::Invalid => None,
        }
    }
}

pub(crate) fn session_region<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    let config = read_source(lookup, CONFIG_VARIABLE, CONFIG_DEFAULT).sections()?;
    let credentials = read_source(lookup, CREDENTIALS_VARIABLE, CREDENTIALS_DEFAULT).sections()?;
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
        .map(region_value);
    lookup.lookup("AWS_DEFAULT_REGION").or(selected)
}

fn read_source<L>(lookup: &L, variable: &str, fallback: &str) -> Source
where
    L: EnvironmentLookup,
{
    let configured = lookup.lookup(variable);
    let text = configured.as_deref().unwrap_or(fallback);
    let path = path::expand(lookup, text);
    if path.as_os_str().is_empty() {
        return Source::Missing;
    }
    let Ok(bytes) = fs::read(path) else {
        return Source::Missing;
    };
    String::from_utf8(bytes).map_or(Source::Invalid, Source::Text)
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
        let tokens = shlex::split(name)?;
        let [_, profile] = tokens.as_slice() else {
            return None;
        };
        return Some(profile.clone());
    }
    if name == DEFAULT_PROFILE {
        return Some(DEFAULT_PROFILE.to_owned());
    }
    None
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

fn region_value(value: &Value) -> String {
    match value {
        Value::Scalar(text) => text.clone(),
        Value::Nested => NESTED_REGION.to_owned(),
    }
}
