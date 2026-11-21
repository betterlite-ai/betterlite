use std::iter::{Copied, Peekable};
use std::slice::Iter;

use indexmap::IndexMap;

use super::error::RecordJsonError;
use super::text::RecordText;
use super::{RecordJson, RecordObject};

// Python stops near its recursion limit, and the upload validation already refused anything deeper than this
pub const MAX_DEPTH: usize = 512;

// The bare words json's scanner accepts; NaN and the infinities stay number literals
const WORDS: [&str; 6] = ["null", "true", "false", "NaN", "Infinity", "-Infinity"];

const ESCAPES: [(char, char); 8] = [
    ('"', '"'),
    ('\\', '\\'),
    ('/', '/'),
    ('b', '\u{8}'),
    ('f', '\u{c}'),
    ('n', '\n'),
    ('r', '\r'),
    ('t', '\t'),
];

enum Frame {
    List(Vec<RecordJson>),
    Object(RecordObject, RecordText),
}

enum Step {
    Done(RecordJson),
    Open(Frame),
}

struct Parser<'line> {
    points: Peekable<Copied<Iter<'line, u32>>>,
}

// The grammar of json's C scanner over decoded code points, with containers kept on a stack instead of the call stack
pub fn parse(code_points: &[u32]) -> Result<RecordJson, RecordJsonError> {
    let mut parser = Parser {
        points: code_points.iter().copied().peekable(),
    };
    let document = parser.document()?;
    parser.skip_whitespace();
    let ended = parser.points.next().is_none();
    ended.then_some(document).ok_or(RecordJsonError::Syntax)
}

impl Parser<'_> {
    fn document(&mut self) -> Result<RecordJson, RecordJsonError> {
        let mut frames = Vec::new();
        loop {
            let value = match self.open(frames.len())? {
                Step::Open(frame) => {
                    frames.push(frame);
                    continue;
                }
                Step::Done(value) => value,
            };
            if let Some(document) = self.attach(&mut frames, value)? {
                return Ok(document);
            }
        }
    }

    // Hands a finished value up the stack until a container wants another value or none is left
    fn attach(
        &mut self,
        frames: &mut Vec<Frame>,
        value: RecordJson,
    ) -> Result<Option<RecordJson>, RecordJsonError> {
        let mut finished = value;
        while let Some(frame) = frames.pop() {
            match self.close(frame, finished)? {
                Step::Done(closed) => finished = closed,
                Step::Open(reopened) => {
                    frames.push(reopened);
                    return Ok(None);
                }
            }
        }
        Ok(Some(finished))
    }

    fn open(&mut self, depth: usize) -> Result<Step, RecordJsonError> {
        self.skip_whitespace();
        let first = self.points.next().ok_or(RecordJsonError::Syntax)?;
        let opened = match char::from_u32(first) {
            Some('[' | '{') if depth >= MAX_DEPTH => return Err(RecordJsonError::Depth),
            Some('[') if self.closes(']') => Step::Done(RecordJson::List(Vec::new())),
            Some('[') => Step::Open(Frame::List(Vec::new())),
            Some('{') if self.closes('}') => Step::Done(RecordJson::Object(IndexMap::new())),
            Some('{') => Step::Open(Frame::Object(IndexMap::new(), self.key()?)),
            Some('"') => Step::Done(RecordJson::Text(self.string()?)),
            Some(character) => Step::Done(self.scalar(character)?),
            None => return Err(RecordJsonError::Syntax),
        };
        Ok(opened)
    }

    // A comma reopens the container for its next value, and its own closer finishes it
    fn close(&mut self, frame: Frame, value: RecordJson) -> Result<Step, RecordJsonError> {
        self.skip_whitespace();
        let separator = self.points.next().and_then(char::from_u32);
        match (frame, separator) {
            (Frame::List(mut items), Some(mark @ (',' | ']'))) => {
                items.push(value);
                if mark == ']' {
                    return Ok(Step::Done(RecordJson::List(items)));
                }
                Ok(Step::Open(Frame::List(items)))
            }
            (Frame::Object(mut members, key), Some(',')) => {
                members.insert(key, value);
                Ok(Step::Open(Frame::Object(members, self.key()?)))
            }
            (Frame::Object(mut members, key), Some('}')) => {
                members.insert(key, value);
                Ok(Step::Done(RecordJson::Object(members)))
            }
            (Frame::List(_) | Frame::Object(..), Some(_) | None) => Err(RecordJsonError::Syntax),
        }
    }

    fn closes(&mut self, close: char) -> bool {
        self.skip_whitespace();
        self.points.next_if_eq(&u32::from(close)).is_some()
    }

    fn key(&mut self) -> Result<RecordText, RecordJsonError> {
        self.skip_whitespace();
        self.points
            .next_if_eq(&u32::from('"'))
            .ok_or(RecordJsonError::Syntax)?;
        let key = self.string()?;
        self.skip_whitespace();
        self.points
            .next_if_eq(&u32::from(':'))
            .ok_or(RecordJsonError::Syntax)?;
        Ok(key)
    }

    fn skip_whitespace(&mut self) {
        while self
            .points
            .next_if(|point| matches!(char::from_u32(*point), Some(' ' | '\t' | '\n' | '\r')))
            .is_some()
        {}
    }

    // The rest of a word whose first character was already read
    fn rest(&mut self, word: &str) -> bool {
        word.chars()
            .skip(1)
            .all(|expected| self.points.next_if_eq(&u32::from(expected)).is_some())
    }

    fn scalar(&mut self, first: char) -> Result<RecordJson, RecordJsonError> {
        let found = WORDS
            .into_iter()
            .find(|word| word.starts_with(first) && self.rest(word));
        if let Some(word) = found {
            return Ok(word_value(word));
        }
        let numeric = first == '-' || first.is_ascii_digit();
        let literal = numeric.then(|| self.number(first));
        literal
            .ok_or(RecordJsonError::Syntax)?
            .map(RecordJson::Number)
    }

    // json's number pattern with ASCII digits only, as the C scanner reads it
    fn number(&mut self, first: char) -> Result<String, RecordJsonError> {
        let mut literal = String::from(first);
        self.digits(&mut literal);
        let integer = literal.strip_prefix('-').unwrap_or(&literal);
        let mut valid =
            integer == "0" || integer.starts_with(|lead: char| ('1'..='9').contains(&lead));
        if self.points.next_if_eq(&u32::from('.')).is_some() {
            literal.push('.');
            valid &= self.digits(&mut literal);
        }
        if let Some(marker) = self.next_of(|found| matches!(found, 'e' | 'E')) {
            literal.push(marker);
            if let Some(sign) = self.next_of(|found| matches!(found, '+' | '-')) {
                literal.push(sign);
            }
            valid &= self.digits(&mut literal);
        }
        valid.then_some(literal).ok_or(RecordJsonError::Syntax)
    }

    fn next_of(&mut self, wanted: fn(&char) -> bool) -> Option<char> {
        let next = self
            .points
            .next_if(|point| char::from_u32(*point).is_some_and(|found| wanted(&found)));
        next.and_then(char::from_u32)
    }

    fn digits(&mut self, literal: &mut String) -> bool {
        let before = literal.len();
        while let Some(digit) = self.next_of(char::is_ascii_digit) {
            literal.push(digit);
        }
        literal.len() > before
    }

    fn string(&mut self) -> Result<RecordText, RecordJsonError> {
        let mut code_points = Vec::new();
        loop {
            let point = self.points.next().ok_or(RecordJsonError::Syntax)?;
            match char::from_u32(point) {
                Some('"') => return Ok(RecordText::from_code_points(&code_points)),
                Some('\\') => code_points.push(self.escape()?),
                Some('\u{0}'..='\u{1f}') => return Err(RecordJsonError::Syntax),
                Some(_) | None => code_points.push(point),
            }
        }
    }

    fn escape(&mut self) -> Result<u32, RecordJsonError> {
        let written = self.points.next().and_then(char::from_u32);
        if written == Some('u') {
            return self.unicode();
        }
        let found = ESCAPES.iter().find(|(escape, _)| Some(*escape) == written);
        found
            .map(|(_, decoded)| u32::from(*decoded))
            .ok_or(RecordJsonError::Syntax)
    }

    // json pairs a high surrogate escape with a low one right after it and keeps either alone otherwise
    fn unicode(&mut self) -> Result<u32, RecordJsonError> {
        let high = self.hex()?;
        let before_pair = self.points.clone();
        let escaped = |parser: &mut Self, mark: char| parser.points.next_if_eq(&u32::from(mark));
        if (0xD800..=0xDBFF).contains(&high)
            && escaped(self, '\\').is_some()
            && escaped(self, 'u').is_some()
        {
            let low = self.hex()?;
            if let Some(Ok(paired)) = char::decode_utf16([high, low]).next() {
                return Ok(u32::from(paired));
            }
        }
        self.points = before_pair;
        Ok(u32::from(high))
    }

    // Exactly four hex digits; from_str_radix alone would also take a sign
    fn hex(&mut self) -> Result<u16, RecordJsonError> {
        let digits: String = self
            .points
            .by_ref()
            .take(4)
            .filter_map(char::from_u32)
            .collect();
        let valid =
            digits.chars().count() == 4 && digits.chars().all(|digit| digit.is_ascii_hexdigit());
        let value = valid
            .then(|| u16::from_str_radix(&digits, 16).ok())
            .flatten();
        value.ok_or(RecordJsonError::Syntax)
    }
}

fn word_value(word: &str) -> RecordJson {
    match word {
        "null" => RecordJson::Null,
        "true" => RecordJson::Bool(true),
        "false" => RecordJson::Bool(false),
        special => RecordJson::Number(special.to_owned()),
    }
}
