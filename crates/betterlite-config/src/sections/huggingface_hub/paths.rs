use std::iter::Peekable;
use std::path::PathBuf;
use std::str::Chars;

use crate::loader::{ConfigError, EnvironmentLookup};

pub(super) fn resolve(
    lookup: &dyn EnvironmentLookup,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), ConfigError> {
    let hf_home_setting = lookup.lookup("HF_HOME").unwrap_or_default();
    let xdg_cache_setting = lookup.lookup("XDG_CACHE_HOME").unwrap_or_default();
    let hf_home = if !hf_home_setting.is_empty() {
        expand_path(&hf_home_setting, lookup)
    } else if !xdg_cache_setting.is_empty() {
        expand_path(&xdg_cache_setting, lookup).join("huggingface")
    } else {
        expand_path("~/.cache/huggingface", lookup)
    };
    let primary_cache = lookup.lookup("HF_HUB_CACHE").unwrap_or_default();
    let hub_cache_setting = if primary_cache.is_empty() {
        lookup.lookup("HUGGINGFACE_HUB_CACHE").unwrap_or_default()
    } else {
        primary_cache
    };
    let hub_cache = if hub_cache_setting.is_empty() {
        hf_home.join("hub")
    } else {
        expand_path(&hub_cache_setting, lookup)
    };
    let staging_setting = lookup.lookup("HUGGINGFACE_CO_STAGING").unwrap_or_default();
    let staging =
        betterlite_env::flag("HUGGINGFACE_CO_STAGING", &staging_setting)?.unwrap_or(false);
    let token_path = if staging {
        expand_path("~/.cache/huggingface_staging/token", lookup)
    } else if lookup
        .lookup("HF_TOKEN_PATH")
        .unwrap_or_default()
        .is_empty()
    {
        hf_home.join("token")
    } else {
        let token_setting = lookup.lookup("HF_TOKEN_PATH").unwrap_or_default();
        expand_path(&token_setting, lookup)
    };
    let stored_tokens_path = token_path
        .parent()
        .unwrap_or(&token_path)
        .join("stored_tokens");
    Ok((hf_home, hub_cache, token_path, stored_tokens_path))
}

pub(super) fn expand_path(value: &str, lookup: &dyn EnvironmentLookup) -> PathBuf {
    let system_home = lookup
        .system_home()
        .and_then(|path| path.to_str().map(str::to_owned));
    let home = lookup.lookup("HOME").or(system_home);
    let user_expanded = expand_user(value, home.as_deref());
    PathBuf::from(expand_variables(&user_expanded, lookup))
}

fn expand_user(value: &str, home: Option<&str>) -> String {
    let Some(home_directory) = home else {
        return value.to_owned();
    };
    let Some(suffix) = value.strip_prefix('~') else {
        return value.to_owned();
    };
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return value.to_owned();
    }
    format!("{}{suffix}", home_directory.trim_end_matches('/'))
}

fn expand_variables(value: &str, lookup: &dyn EnvironmentLookup) -> String {
    let mut expanded = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '$' {
            expanded.push(character);
            continue;
        }
        let variable = parse_variable(&mut chars);
        match variable {
            Some((name, true)) if name.is_empty() => expanded.push_str("${}"),
            Some((name, braced)) => push_variable(&mut expanded, lookup, &name, braced),
            None => expanded.push('$'),
        }
    }
    expanded
}

fn parse_variable(chars: &mut Peekable<Chars<'_>>) -> Option<(String, bool)> {
    if chars.peek() == Some(&'{') {
        chars.next();
        let mut name = String::new();
        for character in chars.by_ref() {
            if character == '}' {
                return Some((name, true));
            }
            name.push(character);
        }
        return Some((format!("{{{name}"), false));
    }
    let mut name = String::new();
    while let Some(character) = chars.peek() {
        if character.is_ascii_alphanumeric() || *character == '_' {
            name.push(*character);
            chars.next();
        } else {
            break;
        }
    }
    (!name.is_empty()).then_some((name, false))
}

fn push_variable(output: &mut String, lookup: &dyn EnvironmentLookup, name: &str, braced: bool) {
    if let Some(value) = lookup.lookup(name) {
        output.push_str(&value);
    } else if braced {
        output.push_str("${");
        output.push_str(name);
        output.push('}');
    } else {
        output.push('$');
        output.push_str(name);
    }
}
