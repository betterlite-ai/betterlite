use chrono::NaiveDate;

use super::super::DateFlags;
use super::resolve::{self, Fields, Label};
use super::tokens::{
    self, HMS_NAMES, MONTH_NAMES, Token, WEEKDAY_NAMES, ampm_of, int_value, is_jump, lookup,
};

const HOUR_MAX: i64 = 23;
const MINUTE_MAX: i64 = 59;
const SECOND_MAX: i64 = 59;
const MICRO_MAX: i64 = 999_999;

pub(super) struct Parser<'a> {
    pub(super) tokens: &'a [Token],
    pub(super) today: NaiveDate,
    pub(super) flags: DateFlags,
    pub(super) fields: Fields,
    pub(super) weekday: Option<i64>,
    pub(super) hour: Option<i64>,
    pub(super) minute: Option<i64>,
    pub(super) second: Option<i64>,
    pub(super) microsecond: Option<i64>,
    pub(super) saw: bool,
}

impl<'a> Parser<'a> {
    pub(super) fn new(tokens: &'a [Token], today: NaiveDate, flags: DateFlags) -> Self {
        Self {
            tokens,
            today,
            flags,
            fields: Fields::default(),
            weekday: None,
            hour: None,
            minute: None,
            second: None,
            microsecond: None,
            saw: false,
        }
    }

    pub(super) fn run(&mut self) -> Option<()> {
        let mut index = 0;
        while index < self.tokens.len() {
            index = self.step(index)?.saturating_add(1);
        }
        Some(())
    }

    fn step(&mut self, index: usize) -> Option<usize> {
        let token = self.tokens.get(index)?;
        match token {
            Token::Number(_) => self.number_token(index),
            Token::Punct('+' | '-') if self.hour.is_some() => self.timezone_group(index),
            Token::Word(_) => self.word_token(index),
            Token::Punct(_) => Some(index),
        }
    }

    fn word_token(&mut self, index: usize) -> Option<usize> {
        let token = self.tokens.get(index)?.clone();
        if let Some(month) = lookup(&token, &MONTH_NAMES) {
            self.fields
                .push_text(&month.to_string(), Some(Label::Month))?;
            return self.month_member(index);
        }
        if let Some(weekday) = lookup(&token, &WEEKDAY_NAMES) {
            self.weekday = Some(weekday);
            self.saw = true;
        } else if ampm_of(&token) && self.hour.is_some() {
            self.saw = true;
        }
        Some(index)
    }

    // "Jan-05-2024" joins a month name to one or two more members with the same separator
    fn month_member(&mut self, index: usize) -> Option<usize> {
        let separator = match self.tokens.get(index.saturating_add(1)) {
            Some(Token::Punct(separator @ ('-' | '/'))) => *separator,
            _ => return Some(index),
        };
        let member = self.tokens.get(index.saturating_add(2))?.clone();
        self.push_member(&member)?;
        let third = index.saturating_add(4);
        if third < self.tokens.len()
            && self.tokens.get(index.saturating_add(3)) == Some(&Token::Punct(separator))
        {
            let token = self.tokens.get(third)?.clone();
            self.push_member(&token)?;
            return Some(third);
        }
        Some(index.saturating_add(2))
    }

    fn push_member(&mut self, token: &Token) -> Option<()> {
        match token {
            Token::Number(text) => self.fields.push_text(text, None),
            Token::Word(_) => {
                let month = lookup(token, &MONTH_NAMES)?;
                self.fields
                    .push_text(&month.to_string(), Some(Label::Month))
            }
            Token::Punct(_) => None,
        }
    }

    fn number_token(&mut self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        let text = match tokens.get(index) {
            Some(Token::Number(text)) => text.as_str(),
            _ => return None,
        };
        let digits = text.chars().count();
        let value = int_value(text)?;
        let following = tokens.get(index.saturating_add(1));
        if following == Some(&Token::Punct(':')) && index.saturating_add(2) < tokens.len() {
            return self.time_group(index);
        }
        if let Some(label) = self.hms_index(index) {
            return self.hms_token(index, label);
        }
        if self.fields.items.len() == 3 && matches!(digits, 2 | 4) && self.hour.is_none() {
            return self.compact_hour(index, digits, value);
        }
        if digits == 6 || (digits > 6 && text.find('.') == Some(6)) {
            return self.compact_time(index);
        }
        if matches!(digits, 8 | 12 | 14) {
            return self.compact_date(index);
        }
        self.number_tail(index, value)
    }

    fn number_tail(&mut self, index: usize, value: i64) -> Option<usize> {
        let tokens = self.tokens;
        let following = tokens.get(index.saturating_add(1));
        if following.is_some_and(|token| matches!(token, Token::Punct('-' | '/' | '.'))) {
            return self.separator_group(index);
        }
        if following.is_none_or(is_jump) {
            let next = tokens.get(index.saturating_add(2));
            if next.is_some_and(ampm_of) {
                let Some(Token::Word(ampm)) = next else {
                    return None;
                };
                self.set_hour(tokens::adjust_hour(value, ampm));
                self.check_time()?;
                return Some(index);
            }
            self.fields.push_value(value, None)?;
            return Some(index);
        }
        if following.is_some_and(ampm_of) && (0..24).contains(&value) {
            let Some(Token::Word(ampm)) = following else {
                return None;
            };
            self.set_hour(tokens::adjust_hour(value, ampm));
            self.check_time()?;
            return Some(index);
        }
        if self.could_be_day(value)? {
            self.fields.push_value(value, None)?;
        }
        Some(index)
    }

    fn number_at(&self, index: usize) -> Option<&'a str> {
        match self.tokens.get(index) {
            Some(Token::Number(text)) => Some(text),
            _ => None,
        }
    }

    fn compact_hour(&mut self, index: usize, digits: usize, value: i64) -> Option<usize> {
        if digits == 2 {
            self.set_hour(value);
        } else {
            let text = self.number_at(index)?;
            self.set_hour(int_value(text.get(..2)?)?);
            self.set_minutes(text.get(2..4)?)?;
        }
        self.check_time()?;
        Some(index)
    }

    fn compact_time(&mut self, index: usize) -> Option<usize> {
        let text = self.number_at(index)?;
        if self.fields.items.is_empty() && !text.contains('.') {
            self.fields.push_text(text.get(..2)?, None)?;
            self.fields.push_text(text.get(2..4)?, None)?;
            self.fields.push_text(text.get(4..6)?, None)?;
            return Some(index);
        }
        self.set_hour(int_value(text.get(..2)?)?);
        self.set_minutes(text.get(2..4)?)?;
        self.set_seconds(text.get(4..)?)?;
        self.check_time()?;
        Some(index)
    }

    fn compact_date(&mut self, index: usize) -> Option<usize> {
        let text = self.number_at(index)?;
        let members = [(..4, Some(Label::Year)), (4..6, None), (6..8, None)];
        for (digits, label) in members {
            self.fields.push_text(text.get(digits)?, label)?;
        }
        if text.len() > 8 {
            self.set_hour(int_value(text.get(8..10)?)?);
            self.minute = Some(int_value(text.get(10..12)?)?);
            if text.len() > 12 {
                self.second = Some(int_value(text.get(12..14)?)?);
            }
            self.check_time()?;
        }
        Some(index)
    }

    fn separator_group(&mut self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        let separator = tokens.get(index.saturating_add(1))?.clone();
        let text = self.number_at(index)?;
        self.fields.push_text(text, None)?;
        let member = match tokens.get(index.saturating_add(2)) {
            Some(token) if !is_jump(token) => token.clone(),
            _ => return Some(index.saturating_add(1)),
        };
        self.push_member(&member)?;
        if tokens.get(index.saturating_add(3)) != Some(&separator) {
            return Some(index.saturating_add(2));
        }
        let third = tokens.get(index.saturating_add(4))?.clone();
        self.push_member(&third)?;
        Some(index.saturating_add(4))
    }

    fn time_group(&mut self, index: usize) -> Option<usize> {
        let [
            Token::Number(hour),
            Token::Punct(':'),
            Token::Number(minutes),
            tail @ ..,
        ] = self.tokens.get(index..)?
        else {
            return None;
        };
        self.set_hour(int_value(hour)?);
        self.set_minutes(minutes)?;
        let mut last = index.saturating_add(2);
        if let [Token::Punct(':'), rest @ ..] = tail {
            let [Token::Number(seconds), ..] = rest else {
                return None;
            };
            self.set_seconds(seconds)?;
            last = index.saturating_add(4);
        }
        self.check_time()?;
        Some(last)
    }

    fn timezone_group(&mut self, index: usize) -> Option<usize> {
        let [Token::Punct(_), Token::Number(text), tail @ ..] = self.tokens.get(index..)? else {
            return None;
        };
        let length = text.chars().count();
        if length != 4 && length > 2 {
            return None;
        }
        if length <= 2 && matches!(tail.first(), Some(Token::Punct(':'))) {
            return Some(index.saturating_add(3));
        }
        Some(index.saturating_add(1))
    }

    fn hms_index(&self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        let after = index.saturating_add(1);
        if tokens
            .get(after)
            .and_then(|token| lookup(token, &HMS_NAMES))
            .is_some()
        {
            return Some(after);
        }
        if tokens.get(after) == Some(&Token::Punct(' '))
            && tokens
                .get(index.saturating_add(2))
                .and_then(|token| lookup(token, &HMS_NAMES))
                .is_some()
        {
            return Some(index.saturating_add(2));
        }
        if index > 0
            && tokens
                .get(index.saturating_sub(1))
                .and_then(|token| lookup(token, &HMS_NAMES))
                .is_some()
        {
            return Some(index.saturating_sub(1));
        }
        if index > 1
            && index == tokens.len().saturating_sub(1)
            && tokens.get(index.saturating_sub(1)) == Some(&Token::Punct(' '))
            && tokens
                .get(index.saturating_sub(2))
                .and_then(|token| lookup(token, &HMS_NAMES))
                .is_some()
        {
            return Some(index.saturating_sub(2));
        }
        None
    }

    fn hms_token(&mut self, index: usize, label: usize) -> Option<usize> {
        let mut kind = lookup(self.tokens.get(label)?, &HMS_NAMES)?;
        if label < index {
            kind = kind.saturating_add(1);
        }
        let text = self.number_at(index)?;
        self.saw = true;
        match kind {
            0 => {
                self.hour = Some(int_value(&text)?);
                if let Some((_, tail)) = text.split_once('.') {
                    self.set_minutes(&format!("0.{tail}"))?;
                }
            }
            1 => self.set_minutes(&text)?,
            _ => self.set_seconds(&text)?,
        }
        self.check_time()?;
        Some(index)
    }

    fn could_be_day(&self, value: i64) -> Option<bool> {
        if self.fields.index(Label::Day).is_some() {
            return Some(false);
        }
        let Some(month_index) = self.fields.index(Label::Month) else {
            return Some((1..=31).contains(&value));
        };
        let month = self.fields.items.get(month_index)?.value;
        let year = match self.fields.index(Label::Year) {
            Some(index) => self.fields.items.get(index)?.value,
            None => 2000,
        };
        let last = i64::from(resolve::days_in_month(year, month)?);
        Some((1..=last).contains(&value))
    }

    fn check_time(&self) -> Option<()> {
        if self
            .hour
            .is_some_and(|hour| !(0..=HOUR_MAX).contains(&hour))
            || self
                .minute
                .is_some_and(|value| !(0..=MINUTE_MAX).contains(&value))
            || self
                .second
                .is_some_and(|value| !(0..=SECOND_MAX).contains(&value))
            || self
                .microsecond
                .is_some_and(|value| !(0..=MICRO_MAX).contains(&value))
        {
            return None;
        }
        Some(())
    }

    fn set_hour(&mut self, value: i64) {
        self.saw = true;
        self.hour = Some(value);
    }

    fn set_minutes(&mut self, text: &str) -> Option<()> {
        let (head, tail) = text.split_once('.').unwrap_or((text, ""));
        self.minute = Some(head.parse().ok()?);
        if !tail.is_empty() && tail.bytes().any(|byte| byte != b'0') {
            let value: i64 = tail.parse().ok()?;
            let scale = 10_i64.checked_pow(u32::try_from(tail.len()).ok()?)?;
            self.second = Some(60_i64.checked_mul(value)?.checked_div(scale)?);
        }
        Some(())
    }

    fn set_seconds(&mut self, text: &str) -> Option<()> {
        let (seconds, tail) = match text.split_once('.') {
            Some((seconds, tail)) => (seconds, Some(tail)),
            None => (text, None),
        };
        self.second = Some(seconds.parse().ok()?);
        let Some(tail) = tail.filter(|tail| !tail.is_empty()) else {
            return Some(());
        };
        let micro: String = tail.chars().chain(std::iter::repeat('0')).take(6).collect();
        self.microsecond = Some(micro.parse().ok()?);
        Some(())
    }
}
