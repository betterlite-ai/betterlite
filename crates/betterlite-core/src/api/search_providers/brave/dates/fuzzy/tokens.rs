use std::mem::take;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Number(String),
    Word(String),
    Punct(char),
}

// the words the source skips; an ordinal suffix needs no entry because any unknown word is skipped
const JUMPS: [&str; 7] = ["at", "on", "and", "ad", "m", "t", "of"];
pub(super) const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
pub(super) const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

// a name matches by its first letters, the way the source accepts "Mar" and "March" for one month
pub(super) fn lookup(token: &Token, names: &[&str]) -> Option<i64> {
    let Token::Word(word) = token else {
        return None;
    };
    let lowered = word.to_ascii_lowercase();
    let index = names.iter().position(|name| lowered.starts_with(name))?;
    i64::try_from(index).ok()
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
