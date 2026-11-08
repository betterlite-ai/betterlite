use std::fs;
use std::path::PathBuf;

use crate::loader::EnvironmentLookup;

// botocore reads the region of the selected profile from AWS_CONFIG_FILE or ~/.aws/config
pub(crate) fn profile_region<L>(lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    let profile = lookup
        .lookup("AWS_PROFILE")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    let path = config_path(lookup)?;
    let source = fs::read_to_string(path).ok()?;
    region_in_section(&source, &profile)
}

fn config_path<L>(lookup: &L) -> Option<PathBuf>
where
    L: EnvironmentLookup,
{
    if let Some(path) = lookup
        .lookup("AWS_CONFIG_FILE")
        .filter(|path| !path.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    let home = lookup
        .lookup("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| lookup.system_home())?;
    Some(home.join(".aws").join("config"))
}

pub(super) fn region_in_section(source: &str, profile: &str) -> Option<String> {
    let wanted = if profile == "default" {
        "default".to_owned()
    } else {
        format!("profile {profile}")
    };
    let mut section = String::new();
    let mut region = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            name.trim().clone_into(&mut section);
            continue;
        }
        if section != wanted || region.is_some() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(['=', ':']) else {
            continue;
        };
        if key.trim() == "region" {
            region = Some(value.trim().to_owned());
        }
    }
    region
}
