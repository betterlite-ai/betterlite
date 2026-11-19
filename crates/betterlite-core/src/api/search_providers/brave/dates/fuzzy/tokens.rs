// a run of digits, a run of letters, or one separator, as the source's lexer splits its input
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Number(String),
    Word(String),
    Punct(char),
}

const JUMPS: [&str; 11] = [
    "at", "on", "and", "ad", "m", "t", "of", "st", "nd", "rd", "th",
];

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
            tokens.push(Token::Word(collect(&chars, start, index)));
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
        tokens.push(Token::Number(collect(chars, start, digits_end)));
        return digits_end;
    }
    let mut end = digits_end;
    while chars
        .get(end)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
    {
        end = end.saturating_add(1);
    }
    let run = collect(chars, start, end);
    if run.matches('.').count() > 1 || run.ends_with(['.', ',']) {
        split_run(&run, tokens);
    } else {
        tokens.push(Token::Number(run.replace(',', ".")));
    }
    end
}

fn collect(chars: &[char], start: usize, end: usize) -> String {
    chars.get(start..end).unwrap_or_default().iter().collect()
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

pub(super) fn month_of(token: &Token) -> Option<i64> {
    let Token::Word(word) = token else {
        return None;
    };
    Some(match word.to_ascii_lowercase().as_str() {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    })
}

pub(super) fn weekday_of(token: &Token) -> Option<u32> {
    let Token::Word(word) = token else {
        return None;
    };
    Some(match word.to_ascii_lowercase().as_str() {
        "mon" | "monday" => 0,
        "tue" | "tues" | "tuesday" => 1,
        "wed" | "wednesday" => 2,
        "thu" | "thur" | "thurs" | "thursday" => 3,
        "fri" | "friday" => 4,
        "sat" | "saturday" => 5,
        "sun" | "sunday" => 6,
        _ => return None,
    })
}

pub(super) fn ampm_of(token: &Token) -> bool {
    let Token::Word(word) = token else {
        return false;
    };
    matches!(word.to_ascii_lowercase().as_str(), "am" | "a" | "pm" | "p")
}

pub(super) fn hms_of(token: &Token) -> Option<u8> {
    let Token::Word(word) = token else {
        return None;
    };
    Some(match word.to_ascii_lowercase().as_str() {
        "h" | "hour" | "hours" => 0,
        "m" | "minute" | "minutes" => 1,
        "s" | "second" | "seconds" => 2,
        _ => return None,
    })
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
    let head = text.split_once('.').map_or(text, |(head, _)| head);
    head.parse().ok()
}
