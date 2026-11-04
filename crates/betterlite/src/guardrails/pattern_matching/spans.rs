use std::ops::Range;

use fancy_regex::Regex;
use unicode_general_category::{GeneralCategory, get_general_category};

use super::{CompiledPattern, PatternError, python_regex};

pub const MAX_TEXT_BYTES: usize = 0x0010_0000;
pub(super) const DIGIT_RANGES: &[(char, char)] = &[
    ('\u{30}', '\u{39}'),
    ('\u{b2}', '\u{b3}'),
    ('\u{b9}', '\u{b9}'),
    ('\u{660}', '\u{669}'),
    ('\u{6f0}', '\u{6f9}'),
    ('\u{7c0}', '\u{7c9}'),
    ('\u{966}', '\u{96f}'),
    ('\u{9e6}', '\u{9ef}'),
    ('\u{a66}', '\u{a6f}'),
    ('\u{ae6}', '\u{aef}'),
    ('\u{b66}', '\u{b6f}'),
    ('\u{be6}', '\u{bef}'),
    ('\u{c66}', '\u{c6f}'),
    ('\u{ce6}', '\u{cef}'),
    ('\u{d66}', '\u{d6f}'),
    ('\u{de6}', '\u{def}'),
    ('\u{e50}', '\u{e59}'),
    ('\u{ed0}', '\u{ed9}'),
    ('\u{f20}', '\u{f29}'),
    ('\u{1040}', '\u{1049}'),
    ('\u{1090}', '\u{1099}'),
    ('\u{1369}', '\u{1371}'),
    ('\u{17e0}', '\u{17e9}'),
    ('\u{1810}', '\u{1819}'),
    ('\u{1946}', '\u{194f}'),
    ('\u{19d0}', '\u{19da}'),
    ('\u{1a80}', '\u{1a89}'),
    ('\u{1a90}', '\u{1a99}'),
    ('\u{1b50}', '\u{1b59}'),
    ('\u{1bb0}', '\u{1bb9}'),
    ('\u{1c40}', '\u{1c49}'),
    ('\u{1c50}', '\u{1c59}'),
    ('\u{2070}', '\u{2070}'),
    ('\u{2074}', '\u{2079}'),
    ('\u{2080}', '\u{2089}'),
    ('\u{2460}', '\u{2468}'),
    ('\u{2474}', '\u{247c}'),
    ('\u{2488}', '\u{2490}'),
    ('\u{24ea}', '\u{24ea}'),
    ('\u{24f5}', '\u{24fd}'),
    ('\u{24ff}', '\u{24ff}'),
    ('\u{2776}', '\u{277e}'),
    ('\u{2780}', '\u{2788}'),
    ('\u{278a}', '\u{2792}'),
    ('\u{a620}', '\u{a629}'),
    ('\u{a8d0}', '\u{a8d9}'),
    ('\u{a900}', '\u{a909}'),
    ('\u{a9d0}', '\u{a9d9}'),
    ('\u{a9f0}', '\u{a9f9}'),
    ('\u{aa50}', '\u{aa59}'),
    ('\u{abf0}', '\u{abf9}'),
    ('\u{ff10}', '\u{ff19}'),
    ('\u{104a0}', '\u{104a9}'),
    ('\u{10a40}', '\u{10a43}'),
    ('\u{10d30}', '\u{10d39}'),
    ('\u{10e60}', '\u{10e68}'),
    ('\u{11052}', '\u{1105a}'),
    ('\u{11066}', '\u{1106f}'),
    ('\u{110f0}', '\u{110f9}'),
    ('\u{11136}', '\u{1113f}'),
    ('\u{111d0}', '\u{111d9}'),
    ('\u{112f0}', '\u{112f9}'),
    ('\u{11450}', '\u{11459}'),
    ('\u{114d0}', '\u{114d9}'),
    ('\u{11650}', '\u{11659}'),
    ('\u{116c0}', '\u{116c9}'),
    ('\u{11730}', '\u{11739}'),
    ('\u{118e0}', '\u{118e9}'),
    ('\u{11950}', '\u{11959}'),
    ('\u{11c50}', '\u{11c59}'),
    ('\u{11d50}', '\u{11d59}'),
    ('\u{11da0}', '\u{11da9}'),
    ('\u{11f50}', '\u{11f59}'),
    ('\u{16a60}', '\u{16a69}'),
    ('\u{16ac0}', '\u{16ac9}'),
    ('\u{16b50}', '\u{16b59}'),
    ('\u{1d7ce}', '\u{1d7ff}'),
    ('\u{1e140}', '\u{1e149}'),
    ('\u{1e2f0}', '\u{1e2f9}'),
    ('\u{1e4f0}', '\u{1e4f9}'),
    ('\u{1e950}', '\u{1e959}'),
    ('\u{1f100}', '\u{1f10a}'),
    ('\u{1fbf0}', '\u{1fbf9}'),
];
const NUMBER_WORDS: &str = "zero|oh|one|two|three|four|five|six|seven|eight|nine";

impl CompiledPattern {
    pub fn find_spans(&self, text: &str) -> Result<Vec<Range<usize>>, PatternError> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(PatternError::TooLarge);
        }
        let keywords = self
            .keyword_regex
            .as_ref()
            .map(|regex| matches(regex, text))
            .transpose()?;
        if keywords.as_ref().is_some_and(Vec::is_empty) {
            return Ok(Vec::new());
        }
        let mut spans = matches(&self.regex, text)?;
        if self.allow_word_numbers {
            spans.extend(word_spans(&self.regex, text)?);
        }
        if let Some(context) = keywords {
            spans.retain(|span| near_keyword(text, span, &context));
        }
        Ok(merge_spans(spans))
    }
}

fn word_spans(regex: &Regex, text: &str) -> Result<Vec<Range<usize>>, PatternError> {
    let mut spans = Vec::new();
    let sequences = python_regex::compile(&format!(
        "(?i)(?<![A-Za-z])(?:{NUMBER_WORDS})(?:[\\s\\-]+(?:{NUMBER_WORDS}))+(?![A-Za-z])"
    ))?;
    let full = python_regex::builder(regex)
        .pattern(format!("(?:{})\\z", regex.as_str()))
        .build()?;
    for matched in sequences.find_iter(text) {
        let found = matched?;
        let Some(digits) = convert_word_numbers(found.as_str())? else {
            continue;
        };
        if full.find(&digits)?.is_some_and(|value| value.start() == 0) {
            spans.push(found.start()..found.end());
        }
    }
    Ok(spans)
}

#[must_use]
pub fn gap_allowed(text: &str) -> bool {
    // Python's word tokenizer counts letters, numbers and underscore, but not combining marks.
    let mut in_word = false;
    let mut words = 0_u8;
    for character in text.chars() {
        if DIGIT_RANGES
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&character))
        {
            return false;
        }
        let word = character == '_'
            || matches!(
                get_general_category(character),
                GeneralCategory::UppercaseLetter
                    | GeneralCategory::LowercaseLetter
                    | GeneralCategory::TitlecaseLetter
                    | GeneralCategory::ModifierLetter
                    | GeneralCategory::OtherLetter
                    | GeneralCategory::DecimalNumber
                    | GeneralCategory::LetterNumber
                    | GeneralCategory::OtherNumber
            );
        if word && !in_word {
            words = words.saturating_add(1);
            if words > 1 {
                return false;
            }
        }
        in_word = word;
    }
    true
}

fn near_keyword(text: &str, value: &Range<usize>, keywords: &[Range<usize>]) -> bool {
    // A farther keyword cannot have a smaller gap than the closest one on either side.
    let after = keywords.partition_point(|keyword| keyword.end <= value.start);
    let nearest = [keywords.get(after.wrapping_sub(1)), keywords.get(after)];
    nearest.into_iter().flatten().any(|keyword| {
        let gap = if value.start >= keyword.end {
            keyword.end..value.start
        } else if keyword.start >= value.end {
            value.end..keyword.start
        } else {
            return true;
        };
        text.get(gap).is_some_and(gap_allowed)
    })
}

#[must_use]
pub fn merge_spans(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.start <= last.end
        {
            last.end = last.end.max(span.end);
            continue;
        }
        merged.push(span);
    }
    merged
}

pub fn convert_word_numbers(sequence: &str) -> Result<Option<String>, PatternError> {
    let tokens = python_regex::compile(NUMBER_WORDS)?;
    let mut digits = String::new();
    for found in tokens.find_iter(sequence) {
        let digit = match found?.as_str().to_ascii_lowercase().as_str() {
            "zero" | "oh" => '0',
            "one" => '1',
            "two" => '2',
            "three" => '3',
            "four" => '4',
            "five" => '5',
            "six" => '6',
            "seven" => '7',
            "eight" => '8',
            "nine" => '9',
            _ => return Ok(None),
        };
        digits.push(digit);
    }
    Ok((!digits.is_empty()).then_some(digits))
}

mod tests;

#[path = "spans/iteration.rs"]
mod iteration;
use iteration::matches;
