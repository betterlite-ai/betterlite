use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::loader::EnvironmentLookup;

mod ini;
mod path;

use ini::{Section, Value};

type Profile = BTreeMap<String, String>;
type ProfileMap = BTreeMap<String, Profile>;

const CONFIG_VARIABLE: &str = "AWS_CONFIG_FILE";
const CONFIG_DEFAULT: &str = "~/.aws/config";
const CREDENTIALS_VARIABLE: &str = "AWS_SHARED_CREDENTIALS_FILE";
const CREDENTIALS_DEFAULT: &str = "~/.aws/credentials";
const DEFAULT_SECTION: &str = "DEFAULT";
const DEFAULT_PROFILE: &str = "default";

pub(crate) fn session_region<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    region_from_texts(
        read_text(lookup, CONFIG_VARIABLE, CONFIG_DEFAULT).as_deref(),
        read_text(lookup, CREDENTIALS_VARIABLE, CREDENTIALS_DEFAULT).as_deref(),
        profile_name(lookup).as_deref(),
    )
}

pub(super) fn region_from_texts(
    config: Option<&str>,
    credentials: Option<&str>,
    profile: Option<&str>,
) -> Option<String> {
    let mut profiles = ProfileMap::new();
    if let Some(source) = config {
        add_config_profiles(&mut profiles, ini::parse(source).ok()?);
    }
    if let Some(source) = credentials {
        overlay_credentials(&mut profiles, ini::parse(source).ok()?);
    }
    profiles
        .get(profile.unwrap_or(DEFAULT_PROFILE))?
        .get("region")
        .cloned()
}

fn read_text<L>(lookup: &L, variable: &str, fallback: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    let configured = lookup.lookup(variable);
    let text = configured.as_deref().unwrap_or(fallback);
    let expanded = path::expand_variables(lookup, text);
    let path = PathBuf::from(path::expand_home(lookup, &expanded));
    if path.as_os_str().is_empty() {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn profile_name<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    lookup
        .lookup("AWS_DEFAULT_PROFILE")
        .or_else(|| lookup.lookup("AWS_PROFILE"))
}

fn add_config_profiles(profiles: &mut ProfileMap, sections: Vec<Section>) {
    for section in sections {
        if section.name.starts_with("profile") {
            let Some(tokens) = ini::shell_split(&section.name) else {
                continue;
            };
            let [_, name] = tokens.as_slice() else {
                continue;
            };
            profiles.insert(name.clone(), scalars(&section.values));
            continue;
        }
        if section.name == DEFAULT_PROFILE {
            profiles.insert(DEFAULT_PROFILE.to_owned(), scalars(&section.values));
        }
    }
}

fn overlay_credentials(profiles: &mut ProfileMap, sections: Vec<Section>) {
    for section in sections {
        if section.name == DEFAULT_SECTION {
            continue;
        }
        let values = scalars(&section.values);
        if let Some(profile) = profiles.get_mut(&section.name) {
            profile.extend(values);
        } else {
            profiles.insert(section.name, values);
        }
    }
}

fn scalars(values: &BTreeMap<String, Value>) -> Profile {
    values
        .iter()
        .filter_map(|(key, value)| match value {
            Value::Scalar(text) => Some((key.clone(), text.clone())),
            Value::Nested => None,
        })
        .collect()
}
