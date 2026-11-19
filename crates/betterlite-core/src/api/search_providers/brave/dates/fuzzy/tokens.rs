use std::mem::take;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Number(String),
    Word(String),
    Punct(char),
}

// the words the source skips; an ordinal suffix needs no entry because any unknown word is skipped
const JUMPS: [&str; 7] = ["at", "on", "and", "ad", "m", "t", "of"];
// the names the source's parserinfo accepts for each month and weekday: the full name and its abridgement
pub(super) const MONTHS: &str =
    "january february march april may june july august september october november december";
pub(super) const WEEKDAYS: &str = "monday tuesday wednesday thursday friday saturday sunday";
// the h/m/s labels and the am/pm names that turn a number into a time of day
const CLOCK_LABELS: [&str; 9] = [
    "h", "hour", "hours", "m", "minute", "minutes", "s", "second", "seconds",
];
const MERIDIEMS: [&str; 4] = ["am", "a", "pm", "p"];

// a name matches whole, as the source's parserinfo does, and yields the position it names
pub(super) fn lookup(token: &Token, names: &str) -> Option<i64> {
    let Token::Word(word) = token else {
        return None;
    };
    let lowered = word.to_ascii_lowercase();
    let index = names.split_ascii_whitespace().position(|name| {
        name == lowered.as_str()
            || name.get(..3) == Some(lowered.as_str())
            || (lowered == "sept" && name == "september")
    })?;
    i64::try_from(index).ok()
}

pub(super) fn is_clock_label(token: &Token) -> bool {
    let Token::Word(word) = token else {
        return false;
    };
    CLOCK_LABELS.contains(&word.to_ascii_lowercase().as_str())
}

pub(super) fn is_meridiem(token: &Token) -> bool {
    let Token::Word(word) = token else {
        return false;
    };
    MERIDIEMS.contains(&word.to_ascii_lowercase().as_str())
}

pub(super) fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut run = String::new();
    let mut number = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch.is_ascii_alphabetic() {
            let digits = ch.is_ascii_digit();
            if !run.is_empty() && digits != number {
                push_run(&mut tokens, &mut run, number);
            }
            number = digits;
            run.push(ch);
            continue;
        }
        push_run(&mut tokens, &mut run, number);
        tokens.push(Token::Punct(ch));
    }
    push_run(&mut tokens, &mut run, number);
    tokens
}

fn push_run(tokens: &mut Vec<Token>, run: &mut String, number: bool) {
    if run.is_empty() {
        return;
    }
    let text = take(run);
    tokens.push(if number {
        Token::Number(text)
    } else {
        Token::Word(text)
    });
}

pub(super) fn is_jump(token: &Token) -> bool {
    match token {
        Token::Punct(ch) => " .,; -/'".contains(*ch),
        Token::Word(word) => JUMPS.contains(&word.to_ascii_lowercase().as_str()),
        Token::Number(_) => false,
    }
}
