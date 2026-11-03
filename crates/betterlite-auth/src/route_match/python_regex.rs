use std::num::ParseIntError;
use std::str::Utf8Error;

use pcre2::bytes::{Regex, RegexBuilder};
use thiserror::Error;

mod characters;
mod scanner;

#[derive(Debug, Error)]
pub(super) enum RegexError {
    #[error("invalid Python route pattern")]
    Syntax,
    #[error("route pattern exceeds compilation limits")]
    Limit,
    #[error("invalid character set")]
    CharacterSet(#[source] Box<regex_syntax::Error>),
    #[error("invalid UTF-8 literal")]
    Utf8(#[source] Utf8Error),
    #[error("invalid character escape")]
    Number(#[source] ParseIntError),
    #[error("cannot compile route pattern")]
    Compile(#[source] pcre2::Error),
}

#[derive(Clone, Copy, Default)]
struct Flags {
    alphabet: Alphabet,
    caseless: bool,
    verbose: bool,
}

pub(super) fn compile(pattern: &str) -> Result<Regex, RegexError> {
    if pattern.len() > 0x4000 {
        return Err(RegexError::Limit);
    }
    let translated = scanner::translate(pattern)?;
    // The outer anchor applies to every alternative, as Python re.match does.
    RegexBuilder::new()
        .utf(true)
        .ucp(true)
        .build(&format!(
            r"(*LIMIT_MATCH=1000000)(*LIMIT_DEPTH=256)\A(?:{translated})"
        ))
        .map_err(RegexError::Compile)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Alphabet {
    Ascii,
    #[default]
    Unicode,
}
