use std::fmt::Display;

use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterPattern,
};
use betterlite_guardrails::pattern_matching::{
    CompiledPattern, blocked_words, check_patterns, compile, masking, spans,
};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Add {
        patterns: Vec<ContentFilterPattern>,
        text: String,
    },
    Spans {
        pattern: String,
        keyword: Option<String>,
        allow_word_numbers: bool,
        text: String,
    },
    Merge {
        spans: Vec<(usize, usize)>,
        text: String,
        tag: String,
    },
    Words {
        content: Option<String>,
    },
    Keyword {
        keyword: String,
        texts: Vec<String>,
    },
    Convert {
        sequence: String,
    },
    Tag {
        format: String,
        name: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Add { patterns, text } => add(patterns, text),
            Input::Spans {
                pattern,
                keyword,
                allow_word_numbers,
                text,
            } => {
                let mut entry = custom(pattern)?;
                entry.keyword_regex = keyword
                    .as_deref()
                    .map(custom)
                    .transpose()?
                    .map(|value| value.regex);
                entry.allow_word_numbers = *allow_word_numbers;
                let found = entry.find_spans(text).map_err(failure)?;
                Ok(json!(character_spans(text, &found)))
            }
            Input::Merge {
                spans: ranges,
                text,
                tag,
            } => {
                let merged =
                    spans::merge_spans(ranges.iter().map(|(start, end)| *start..*end).collect());
                Ok(
                    json!({"spans": character_spans(text, &merged), "masked": masking::mask_spans(text, &merged, tag).map_err(failure)?}),
                )
            }
            Input::Words { content } => words(content.as_deref()),
            Input::Keyword { keyword, texts } => keyword_matches(keyword, texts),
            Input::Convert { sequence } => Ok(json!(
                spans::convert_word_numbers(sequence).map_err(failure)?
            )),
            Input::Tag { format, name } => Ok(json!(
                masking::redaction_tag(format, name).map_err(failure)?
            )),
        }
    }
}

fn add(configs: &[ContentFilterPattern], text: &str) -> HelperResult {
    let patterns: Vec<_> = configs
        .iter()
        .map(compile)
        .collect::<Result<_, _>>()
        .map_err(failure)?;
    let mut entries = Vec::new();
    for pattern in &patterns {
        let found = pattern.find_spans(text).map_err(failure)?;
        entries.push(json!({
            "name": pattern.pattern_name, "action": pattern.action,
            "allow_word_numbers": pattern.allow_word_numbers,
            "has_keyword": pattern.keyword_regex.is_some(),
            "spans": character_spans(text, &found),
        }));
    }
    Ok(json!({"patterns": entries, "first": check_patterns(text, &patterns).map_err(failure)?}))
}

fn custom(source: &str) -> Result<CompiledPattern, Box<HelperError>> {
    compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source.to_owned()),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .map_err(failure)
}

fn character_spans(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .map(|range| {
            (
                text.get(..range.start).unwrap_or_default().chars().count(),
                text.get(..range.end).unwrap_or_default().chars().count(),
            )
        })
        .collect()
}

fn words(content: Option<&str>) -> HelperResult {
    let temporary = tempfile::tempdir().map_err(failure)?;
    let path = temporary.path().join("blocked.yaml");
    if let Some(yaml) = content {
        std::fs::write(&path, yaml).map_err(failure)?;
    }
    let loaded = blocked_words::load(&path).map_err(failure)?;
    Ok(json!(loaded))
}

fn keyword_matches(keyword: &str, texts: &[String]) -> HelperResult {
    let source = masking::keyword_pattern(keyword);
    let entry = custom(&source)?;
    let found: Vec<_> = texts
        .iter()
        .map(|text| entry.regex.is_match(text))
        .collect::<Result<_, _>>()
        .map_err(failure)?;
    Ok(json!({"pattern": source, "matches": found}))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
