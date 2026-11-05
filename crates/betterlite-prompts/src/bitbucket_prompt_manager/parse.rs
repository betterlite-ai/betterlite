use thiserror::Error;

use super::template::{BitBucketPromptTemplate, TemplateError};
use super::yaml_basic::{BasicYamlError, parse_yaml_basic};
use crate::arguments::json::{Json, Object};

const DELIMITER: &str = "---";

// LiteLLM reads frontmatter with PyYAML and drops to the basic parser only when PyYAML is not installed
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YamlParser {
    #[default]
    Full,
    Basic,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("the frontmatter must be a mapping")]
    NotAMapping,
    #[error(transparent)]
    Basic(#[from] BasicYamlError),
    #[error(transparent)]
    Template(#[from] TemplateError),
}

fn parse_full_yaml(frontmatter: &str) -> Result<Object, ParseError> {
    // invalid YAML erases the metadata instead of failing the load
    let Ok(value) = serde_saphyr::from_str::<Json>(frontmatter) else {
        return Ok(Object::new());
    };
    if !value.is_truthy() {
        return Ok(Object::new());
    }
    match value {
        Json::Object(object) => Ok(object),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ParseError::NotAMapping)
        }
    }
}

pub fn parse_prompt_file(
    content: &str,
    prompt_id: &str,
    parser: YamlParser,
) -> Result<BitBucketPromptTemplate, ParseError> {
    let mut parts = content.splitn(3, DELIMITER);
    let (frontmatter, template_content) =
        match (content.starts_with(DELIMITER), parts.nth(1), parts.next()) {
            (true, Some(frontmatter), Some(template)) => (frontmatter.trim(), template.trim()),
            _ => ("", content),
        };
    let metadata = match (frontmatter.is_empty(), parser) {
        (true, YamlParser::Full | YamlParser::Basic) => Object::new(),
        (false, YamlParser::Full) => parse_full_yaml(frontmatter)?,
        (false, YamlParser::Basic) => parse_yaml_basic(frontmatter)?,
    };
    Ok(BitBucketPromptTemplate::new(
        prompt_id.to_owned(),
        template_content.to_owned(),
        metadata,
        None,
    )?)
}
