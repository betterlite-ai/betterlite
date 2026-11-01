use fancy_regex::Regex;
use thiserror::Error;

use crate::arguments::json::{Json, Object};

// Mirrors Python's re.DOTALL match: the remainder after the matched delimiter is the template content.
const FRONTMATTER_PATTERN: &str = r"(?s)^---\s*\n(.*?)\n---\s*\n";

#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("Invalid YAML frontmatter: {source}")]
    InvalidYaml {
        #[source]
        source: serde_saphyr::Error,
    },
    #[error("YAML frontmatter must be a mapping")]
    NotAMapping,
}

pub(super) fn parse_frontmatter(content: &str) -> Result<(Object, String), FrontmatterError> {
    let Ok(pattern) = Regex::new(FRONTMATTER_PATTERN) else {
        return Ok((Object::new(), content.to_owned()));
    };
    let found = pattern.captures(content).ok().flatten();
    let Some(captures) = found else {
        return Ok((Object::new(), content.to_owned()));
    };
    let (Some(yaml_block), Some(whole_match)) = (captures.get(1), captures.get(0)) else {
        return Ok((Object::new(), content.to_owned()));
    };

    let frontmatter = parse_metadata(yaml_block.as_str())?;
    let template_content = content
        .get(whole_match.end()..)
        .unwrap_or_default()
        .to_owned();
    Ok((frontmatter, template_content))
}

fn parse_metadata(yaml: &str) -> Result<Object, FrontmatterError> {
    let value: Json =
        serde_saphyr::from_str(yaml).map_err(|source| FrontmatterError::InvalidYaml { source })?;
    if !value.is_truthy() {
        return Ok(Object::new());
    }
    match value {
        Json::Object(object) => Ok(object),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(FrontmatterError::NotAMapping)
        }
    }
}

mod tests;
