use super::frontmatter::{self, FrontmatterError};
use crate::arguments::json::{Json, Object};

pub fn dotprompt_content_to_json(dotprompt_content: &str) -> Result<Object, FrontmatterError> {
    let (metadata, content) = frontmatter::parse_frontmatter(dotprompt_content)?;
    let mut result = Object::new();
    result.insert("content".to_owned(), Json::Text(content.trim().to_owned()));
    result.insert("metadata".to_owned(), Json::Object(metadata));
    Ok(result)
}

mod tests;
