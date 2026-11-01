use indexmap::IndexMap;

use super::ConfigFileError;

const MAX_INTERPOLATION_DEPTH: u32 = 10;

pub(super) fn interpolate_section(
    section: &str,
    merged: &IndexMap<String, String>,
) -> Result<IndexMap<String, String>, ConfigFileError> {
    let mut result = IndexMap::new();
    for (option, raw) in merged {
        let value = interpolate_value(section, option, raw, merged, 1)?;
        result.insert(option.clone(), value);
    }
    Ok(result)
}

fn interpolate_value(
    section: &str,
    option: &str,
    raw: &str,
    map: &IndexMap<String, String>,
    depth: u32,
) -> Result<String, ConfigFileError> {
    if depth > MAX_INTERPOLATION_DEPTH {
        return Err(ConfigFileError::InterpolationDepth {
            section: section.to_owned(),
            option: option.to_owned(),
            depth,
        });
    }
    let mut result = String::new();
    let mut rest = raw;
    while let Some(position) = rest.find('%') {
        let literal = rest.get(..position).unwrap_or_default();
        let remainder = rest.get(position..).unwrap_or_default();
        result.push_str(literal);
        let (replacement, next_rest) = consume_escape(section, option, map, remainder, depth)?;
        result.push_str(&replacement);
        rest = next_rest;
    }
    result.push_str(rest);
    Ok(result)
}

fn consume_escape<'input>(
    section: &str,
    option: &str,
    map: &IndexMap<String, String>,
    remainder: &'input str,
    depth: u32,
) -> Result<(String, &'input str), ConfigFileError> {
    match remainder.as_bytes().get(1).copied() {
        Some(b'%') => Ok((String::from("%"), remainder.get(2..).unwrap_or_default())),
        Some(b'(') => interpolate_reference(section, option, map, remainder, depth),
        _ => Err(ConfigFileError::InterpolationSyntax {
            section: section.to_owned(),
            option: option.to_owned(),
        }),
    }
}

fn interpolate_reference<'input>(
    section: &str,
    option: &str,
    map: &IndexMap<String, String>,
    remainder: &'input str,
    depth: u32,
) -> Result<(String, &'input str), ConfigFileError> {
    let after_paren = remainder.get(2..).unwrap_or_default();
    let Some(close) = after_paren.find(')') else {
        return Err(ConfigFileError::InterpolationSyntax {
            section: section.to_owned(),
            option: option.to_owned(),
        });
    };
    let name = after_paren.get(..close).unwrap_or_default().to_lowercase();
    let after_close = after_paren.get(close..).unwrap_or_default();
    let Some(tail) = after_close
        .strip_prefix(')')
        .and_then(|rest| rest.strip_prefix('s'))
    else {
        return Err(ConfigFileError::InterpolationSyntax {
            section: section.to_owned(),
            option: option.to_owned(),
        });
    };
    if name.is_empty() {
        return Err(ConfigFileError::InterpolationSyntax {
            section: section.to_owned(),
            option: option.to_owned(),
        });
    }
    let Some(value) = map.get(&name) else {
        return Err(ConfigFileError::InterpolationMissingName {
            section: section.to_owned(),
            option: option.to_owned(),
        });
    };
    let substituted = if value.contains('%') {
        interpolate_value(section, option, value, map, depth.saturating_add(1))?
    } else {
        value.clone()
    };
    Ok((substituted, tail))
}
