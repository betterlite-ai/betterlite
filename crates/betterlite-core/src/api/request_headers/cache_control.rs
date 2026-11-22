use indexmap::IndexMap;

use crate::api::request_headers::error::RequestHeadersError;

// LiteLLM splits on a comma and a space, so "no-cache,max-age=0" stays one directive
const DIRECTIVE_SEPARATOR: &str = ", ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheDirective {
    Flag,
    Value(String),
}

pub fn parse_cache_control(
    cache_control: &str,
) -> Result<IndexMap<String, CacheDirective>, RequestHeadersError> {
    let mut directives = IndexMap::new();
    for directive in cache_control.split(DIRECTIVE_SEPARATOR) {
        let (key, parsed) = match directive.split_once('=') {
            None => (directive, CacheDirective::Flag),
            Some((_key, value)) if value.contains('=') => {
                return Err(RequestHeadersError::CacheControl {
                    directive: directive.to_owned(),
                });
            }
            Some((key, value)) => (key, CacheDirective::Value(value.to_owned())),
        };
        directives.insert(key.to_owned(), parsed);
    }
    Ok(directives)
}
