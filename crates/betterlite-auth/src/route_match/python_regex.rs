use std::num::ParseIntError;
use std::str::Utf8Error;

use pcre2::bytes::{Regex, RegexBuilder};
use regex_syntax::hir::ClassUnicode;
use thiserror::Error;

mod characters;
mod encoded;
mod scanner;

#[derive(Debug, Error)]
pub(super) enum RegexError {
    #[error("invalid Python route pattern")]
    Syntax,
    #[error("route pattern exceeds resource limits")]
    Limit,
    #[error("invalid character set")]
    CharacterSet(#[source] Box<regex_syntax::Error>),
    #[error("invalid UTF-8 literal")]
    Utf8(#[source] Utf8Error),
    #[error("invalid character escape")]
    Number(#[source] ParseIntError),
    #[error("cannot compile route pattern")]
    Compile(#[source] pcre2::Error),
    #[error("cannot evaluate route pattern")]
    Match(#[source] pcre2::Error),
}

#[derive(Clone, Copy, Default)]
struct Flags {
    alphabet: Alphabet,
    caseless: bool,
    verbose: bool,
    newlines: Newlines,
}

pub(super) fn compile(pattern: &str) -> Result<Pattern, RegexError> {
    if pattern.len() > 0x4000 {
        return Err(RegexError::Limit);
    }
    let initial = scanner::translate(pattern, false)?;
    let translated = if initial.folded_refs {
        scanner::translate(pattern, true)?
    } else {
        initial
    };
    let source = if let Some(classes) = &translated.classes {
        encoded::pattern(&translated.source, classes.len())?
    } else {
        translated.source
    };
    // The outer anchor applies to every alternative, as Python re.match does.
    let engine = RegexBuilder::new()
        .utf(true)
        .ucp(true)
        .build(&format!(
            r"(*LIMIT_MATCH=1000000)(*LIMIT_DEPTH=256)\A(?:{source})"
        ))
        .map_err(RegexError::Compile)?;
    Ok(Pattern {
        engine,
        classes: translated.classes,
    })
}

pub(super) struct Pattern {
    engine: Regex,
    classes: Option<Vec<ClassUnicode>>,
}

impl Pattern {
    pub(super) fn is_match(&self, text: &str) -> Result<bool, RegexError> {
        let encoded_text;
        let subject = if let Some(classes) = &self.classes {
            encoded_text = encoded::text(text, classes)?;
            encoded_text.as_bytes()
        } else {
            text.as_bytes()
        };
        self.engine.is_match(subject).map_err(RegexError::Match)
    }
}

struct Translation {
    source: String,
    classes: Option<Vec<ClassUnicode>>,
    folded_refs: bool,
}

#[derive(Clone, Copy, Default)]
struct Newlines {
    dot: bool,
    anchors: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Alphabet {
    Ascii,
    #[default]
    Unicode,
}
