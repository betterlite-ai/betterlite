use regex::{Regex, escape};

#[derive(Debug, thiserror::Error)]
pub enum TagPatternError {
    #[error("the configured tag pattern {pattern} could not be compiled")]
    Compile {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

// PatternMatchRouter._pattern_to_regex escapes every metacharacter and turns each star into (.*), and re.match anchors at the start only
pub fn tag_matches_wildcard_pattern(
    tags: &[String],
    configured: &str,
) -> Result<bool, TagPatternError> {
    let regex_pattern = escape(configured).replace("\\*", "(.*)");
    let regex =
        Regex::new(&format!("\\A{regex_pattern}")).map_err(|source| TagPatternError::Compile {
            pattern: configured.to_owned(),
            source,
        })?;
    Ok(tags.iter().any(|tag| regex.is_match(tag)))
}

#[path = "tag_wildcard_matching/tests.rs"]
mod tests;
