use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Chars;

use crate::loader::{ConfigError, EnvironmentLookup};

pub(super) fn resolve(
    lookup: &dyn EnvironmentLookup,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), ConfigError> {
    let hf_home = resolve_hf_home(lookup)?;
    let hub_cache = resolve_hub_cache(lookup, &hf_home)?;
    let (token_path, stored_tokens_path) = resolve_token_paths(lookup, &hf_home)?;
    Ok((hf_home, hub_cache, token_path, stored_tokens_path))
}

fn resolve_hf_home(lookup: &dyn EnvironmentLookup) -> Result<PathBuf, ConfigError> {
    let configured = lookup.lookup("HF_HOME").filter(|value| !value.is_empty());
    if let Some(value) = configured {
        return expand_path(&value, "HF_HOME", lookup);
    }
    let xdg = lookup
        .lookup("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty());
    match xdg {
        Some(value) => Ok(expand_path(&value, "XDG_CACHE_HOME", lookup)?.join("huggingface")),
        None => expand_path("~/.cache/huggingface", "HF_HOME", lookup),
    }
}

fn resolve_hub_cache(
    lookup: &dyn EnvironmentLookup,
    hf_home: &Path,
) -> Result<PathBuf, ConfigError> {
    let configured = lookup
        .lookup("HF_HUB_CACHE")
        .filter(|value| !value.is_empty());
    let (setting_name, setting) = match configured {
        Some(value) => ("HF_HUB_CACHE", value),
        None => match lookup
            .lookup("HUGGINGFACE_HUB_CACHE")
            .filter(|value| !value.is_empty())
        {
            Some(value) => ("HUGGINGFACE_HUB_CACHE", value),
            None => return Ok(hf_home.join("hub")),
        },
    };
    expand_path(&setting, setting_name, lookup)
}

fn resolve_token_paths(
    lookup: &dyn EnvironmentLookup,
    hf_home: &Path,
) -> Result<(PathBuf, PathBuf), ConfigError> {
    let staging_setting = lookup.lookup("HUGGINGFACE_CO_STAGING").unwrap_or_default();
    let staging =
        betterlite_env::flag("HUGGINGFACE_CO_STAGING", &staging_setting)?.unwrap_or(false);
    let configured_token_path = match lookup
        .lookup("HF_TOKEN_PATH")
        .filter(|setting| !setting.is_empty())
    {
        Some(setting) => expand_path(&setting, "HF_TOKEN_PATH", lookup)?,
        None => hf_home.join("token"),
    };
    let stored_tokens_path = configured_token_path
        .parent()
        .unwrap_or(&configured_token_path)
        .join("stored_tokens");
    let token_path = if staging {
        expand_path(
            "~/.cache/huggingface_staging/token",
            "HF_TOKEN_PATH",
            lookup,
        )?
    } else {
        configured_token_path
    };
    Ok((token_path, stored_tokens_path))
}

pub(super) fn expand_path(
    value: &str,
    variable: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<PathBuf, ConfigError> {
    let system_home = lookup
        .system_home()
        .and_then(|path| path.to_str().map(str::to_owned));
    let home = lookup.lookup("HOME").or(system_home);
    let user_expanded = expand_user(value, home.as_deref(), variable)?;
    Ok(PathBuf::from(expand_variables(&user_expanded, lookup)))
}

fn expand_user(value: &str, home: Option<&str>, variable: &str) -> Result<String, ConfigError> {
    let Some(suffix) = value.strip_prefix('~') else {
        return Ok(value.to_owned());
    };
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return Err(ConfigError::Unsupported {
            paths: vec![format!(
                "{variable}: named-user tilde paths are unsupported"
            )],
        });
    }
    let Some(home_directory) = home else {
        return Ok(value.to_owned());
    };
    Ok(format!("{}{suffix}", home_directory.trim_end_matches('/')))
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
