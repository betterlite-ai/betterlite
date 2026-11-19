#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Number(String),
    Word(String),
    Punct(char),
}

// the words the source skips; an ordinal suffix needs no entry because any unknown word is skipped
const JUMPS: [&str; 7] = ["at", "on", "and", "ad", "m", "t", "of"];

pub(super) const MONTH_NAMES: [(&str, i64); 24] = [
    ("jan", 1),
    ("january", 1),
    ("feb", 2),
    ("february", 2),
    ("mar", 3),
    ("march", 3),
    ("apr", 4),
    ("april", 4),
    ("may", 5),
    ("jun", 6),
    ("june", 6),
    ("jul", 7),
    ("july", 7),
    ("aug", 8),
    ("august", 8),
    ("sep", 9),
    ("sept", 9),
    ("september", 9),
    ("oct", 10),
    ("october", 10),
    ("nov", 11),
    ("november", 11),
    ("dec", 12),
    ("december", 12),
];

pub(super) const WEEKDAY_NAMES: [(&str, i64); 17] = [
    ("mon", 0),
    ("monday", 0),
    ("tue", 1),
    ("tues", 1),
    ("tuesday", 1),
    ("wed", 2),
    ("wednesday", 2),
    ("thu", 3),
    ("thur", 3),
    ("thurs", 3),
    ("thursday", 3),
    ("fri", 4),
    ("friday", 4),
    ("sat", 5),
    ("saturday", 5),
    ("sun", 6),
    ("sunday", 6),
];

pub(super) const HMS_NAMES: [(&str, i64); 9] = [
    ("h", 0),
    ("hour", 0),
    ("hours", 0),
    ("m", 1),
    ("minute", 1),
    ("minutes", 1),
    ("s", 2),
    ("second", 2),
    ("seconds", 2),
];

pub(super) fn lookup(token: &Token, names: &[(&str, i64)]) -> Option<i64> {
    let Token::Word(word) = token else {
        return None;
    };
    let lowered = word.to_ascii_lowercase();
    names
        .iter()
        .find(|(name, _)| *name == lowered.as_str())
        .map(|(_, value)| *value)
}

pub(super) fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(&first) = chars.get(index) {
        if first.is_ascii_digit() {
            index = number_run(&chars, index, &mut tokens);
        } else if first.is_ascii_alphabetic() {
            let start = index;
            while chars.get(index).is_some_and(|c| c.is_ascii_alphabetic()) {
                index = index.saturating_add(1);
            }
            let word: String = chars.get(start..index).unwrap_or_default().iter().collect();
            tokens.push(Token::Word(word));
        } else if first.is_whitespace() {
            tokens.push(Token::Punct(' '));
            index = index.saturating_add(1);
        } else {
            tokens.push(Token::Punct(first));
            index = index.saturating_add(1);
        }
    }
    tokens
}

fn number_run(chars: &[char], start: usize, tokens: &mut Vec<Token>) -> usize {
    let mut index = start;
    while chars.get(index).is_some_and(char::is_ascii_digit) {
        index = index.saturating_add(1);
    }
    let digits_end = index;
    let joins = match chars.get(index).copied() {
        Some('.') => chars
            .get(index.saturating_add(1))
            .is_some_and(char::is_ascii_digit),
        Some(',') => {
            digits_end.saturating_sub(start) >= 2
                && chars
                    .get(index.saturating_add(1))
                    .is_some_and(char::is_ascii_digit)
        }
        _ => false,
    };
    if !joins {
        let digits: String = chars
            .get(start..digits_end)
            .unwrap_or_default()
            .iter()
            .collect();
        tokens.push(Token::Number(digits));
        return digits_end;
    }
    let mut end = digits_end;
    while chars
        .get(end)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
    {
        end = end.saturating_add(1);
    }
    let run: String = chars.get(start..end).unwrap_or_default().iter().collect();
    if run.matches('.').count() > 1 || run.ends_with(['.', ',']) {
        split_run(&run, tokens);
    } else {
        tokens.push(Token::Number(run.replace(',', ".")));
    }
    end
}

fn split_run(run: &str, tokens: &mut Vec<Token>) {
    let mut piece = String::new();
    for ch in run.chars() {
        if ch == '.' || ch == ',' {
            if !piece.is_empty() {
                tokens.push(Token::Number(std::mem::take(&mut piece)));
            }
            tokens.push(Token::Punct(ch));
        } else {
            piece.push(ch);
        }
    }
    if !piece.is_empty() {
        tokens.push(Token::Number(piece));
    }
}

pub(super) fn is_jump(token: &Token) -> bool {
    match token {
        Token::Punct(ch) => " .,; -/'".contains(*ch),
        Token::Word(word) => JUMPS.contains(&word.to_ascii_lowercase().as_str()),
        Token::Number(_) => false,
    }
}

pub(super) fn ampm_of(token: &Token) -> bool {
    let Token::Word(word) = token else {
        return false;
    };
    matches!(word.to_ascii_lowercase().as_str(), "am" | "a" | "pm" | "p")
}

pub(super) fn adjust_hour(hour: i64, ampm: &str) -> i64 {
    let lowered = ampm.to_ascii_lowercase();
    if hour < 12 && lowered.starts_with('p') {
        return hour.saturating_add(12);
    }
    if hour == 12 && lowered.starts_with('a') {
        return 0;
    }
    hour
}

pub(super) fn int_value(text: &str) -> Option<i64> {
    match text.split_once('.') {
        Some((head, _)) => head.parse().ok(),
        None => text.parse().ok(),
    }
}
