use chrono::{Datelike as _, NaiveDate};

use super::super::DateFlags;
use super::resolve::{self, Fields, Label};
use super::tokens::{self, Token, ampm_of, hms_of, int_value, is_jump, month_of, weekday_of};

const HOUR_MAX: i64 = 23;
const MINUTE_MAX: i64 = 59;
const SECOND_MAX: i64 = 59;
const MICRO_MAX: i64 = 999_999;

pub(super) struct Parser<'a> {
    pub(super) tokens: &'a [Token],
    pub(super) today: NaiveDate,
    pub(super) flags: DateFlags,
    pub(super) fields: Fields,
    pub(super) weekday: Option<u32>,
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
        if let Some(month) = month_of(&token) {
            self.fields
                .push_text(&month.to_string(), Some(Label::Month))?;
            let separator = match self.tokens.get(index.saturating_add(1)) {
                Some(Token::Punct(separator @ ('-' | '/'))) => Some(*separator),
                _ => None,
            };
            let Some(separator) = separator else {
                return Some(index);
            };
            let member = self.tokens.get(index.saturating_add(2))?.clone();
            self.push_member(&member)?;
            if index.saturating_add(4) < self.tokens.len()
                && self.tokens.get(index.saturating_add(3)) == Some(&Token::Punct(separator))
            {
                let third = self.tokens.get(index.saturating_add(4))?.clone();
                self.push_member(&third)?;
                return Some(index.saturating_add(4));
            }
            return Some(index.saturating_add(2));
        }
        if let Some(weekday) = weekday_of(&token) {
            self.weekday = Some(weekday);
            self.saw = true;
        } else if ampm_of(&token) && self.hour.is_some() {
            self.saw = true;
        }
        Some(index)
    }

    fn push_member(&mut self, token: &Token) -> Option<()> {
        match token {
            Token::Number(text) => self.fields.push_text(text, None),
            Token::Word(_) => {
                let month = month_of(token)?;
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
        if self.fields.len() == 3 && matches!(digits, 2 | 4) && self.hour.is_none() {
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

    fn compact_hour(&mut self, index: usize, digits: usize, value: i64) -> Option<usize> {
        let text = match self.tokens.get(index) {
            Some(Token::Number(text)) => text.as_str(),
            _ => return None,
        };
        if digits == 2 {
            self.set_hour(value);
        } else {
            self.set_hour(int_value(text.get(..2)?)?);
            self.set_minutes(text.get(2..4)?)?;
        }
        self.check_time()?;
        Some(index)
    }

    fn compact_time(&mut self, index: usize) -> Option<usize> {
        let text = match self.tokens.get(index) {
            Some(Token::Number(text)) => text.as_str(),
            _ => return None,
        };
        if self.fields.is_empty() && !text.contains('.') {
            self.fields.push_text(text.get(..2)?, None)?;
            self.fields.push_text(text.get(2..4)?, None)?;
            self.fields.push_text(text.get(4..6)?, None)?;
        } else {
            self.set_hour(int_value(text.get(..2)?)?);
            self.set_minutes(text.get(2..4)?)?;
            self.set_seconds(text.get(4..)?)?;
            self.check_time()?;
        }
        Some(index)
    }

    fn compact_date(&mut self, index: usize) -> Option<usize> {
        let text = match self.tokens.get(index) {
            Some(Token::Number(text)) => text.as_str(),
            _ => return None,
        };
        self.fields.push_text(text.get(..4)?, Some(Label::Year))?;
        self.fields.push_text(text.get(4..6)?, None)?;
        self.fields.push_text(text.get(6..8)?, None)?;
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
        let text = match tokens.get(index) {
            Some(Token::Number(text)) => text.clone(),
            _ => return None,
        };
        self.fields.push_text(&text, None)?;
        let mut last = index.saturating_add(1);
        let member = match tokens.get(index.saturating_add(2)) {
            Some(token) if !is_jump(token) => Some(token.clone()),
            _ => None,
        };
        if let Some(member) = member {
            self.push_member(&member)?;
            last = index.saturating_add(2);
            if tokens.get(index.saturating_add(3)) == Some(&separator) {
                let third = tokens.get(index.saturating_add(4))?.clone();
                self.push_member(&third)?;
                last = index.saturating_add(4);
            }
        }
        Some(last)
    }

    fn time_group(&mut self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        let hour = match tokens.get(index) {
            Some(Token::Number(text)) => int_value(text)?,
            _ => return None,
        };
        self.set_hour(hour);
        let minutes = match tokens.get(index.saturating_add(2)) {
            Some(Token::Number(text)) => text.clone(),
            _ => return None,
        };
        self.set_minutes(&minutes)?;
        let mut last = index.saturating_add(2);
        if tokens.get(index.saturating_add(3)) == Some(&Token::Punct(':')) {
            let seconds = match tokens.get(index.saturating_add(4)) {
                Some(Token::Number(text)) => text.clone(),
                _ => return None,
            };
            self.set_seconds(&seconds)?;
            last = index.saturating_add(4);
        }
        self.check_time()?;
        Some(last)
    }

    fn timezone_group(&mut self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        let text = match tokens.get(index.saturating_add(1)) {
            Some(Token::Number(text)) => text.as_str(),
            _ => return None,
        };
        let length = text.chars().count();
        if length != 4 && length > 2 {
            return None;
        }
        if length <= 2 && tokens.get(index.saturating_add(2)) == Some(&Token::Punct(':')) {
            return Some(index.saturating_add(3));
        }
        Some(index.saturating_add(1))
    }

    fn hms_index(&self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        if tokens
            .get(index.saturating_add(1))
            .and_then(hms_of)
            .is_some()
        {
            return Some(index.saturating_add(1));
        }
        if tokens.get(index.saturating_add(1)) == Some(&Token::Punct(' '))
            && tokens
                .get(index.saturating_add(2))
                .and_then(hms_of)
                .is_some()
        {
            return Some(index.saturating_add(2));
        }
        if index > 0
            && tokens
                .get(index.saturating_sub(1))
                .and_then(hms_of)
                .is_some()
        {
            return Some(index.saturating_sub(1));
        }
        if index > 1
            && index == tokens.len().saturating_sub(1)
            && tokens.get(index.saturating_sub(1)) == Some(&Token::Punct(' '))
            && tokens
                .get(index.saturating_sub(2))
                .and_then(hms_of)
                .is_some()
        {
            return Some(index.saturating_sub(2));
        }
        None
    }

    fn hms_token(&mut self, index: usize, label: usize) -> Option<usize> {
        let mut kind = hms_of(self.tokens.get(label)?)?;
        if label < index {
            kind = kind.saturating_add(1);
        }
        let text = match self.tokens.get(index) {
            Some(Token::Number(text)) => text.clone(),
            _ => return None,
        };
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
        if self.fields.has(Label::Day) {
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
        let (head, tail) = text.split_once('.').unwrap_or((text, ""));
        self.second = Some(head.parse().ok()?);
        if !tail.is_empty() {
            let micro: String = tail.chars().chain(std::iter::repeat('0')).take(6).collect();
            self.microsecond = Some(micro.parse().ok()?);
        }
        Some(())
    }
}
