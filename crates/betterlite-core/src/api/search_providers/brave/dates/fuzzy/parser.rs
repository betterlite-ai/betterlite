use chrono::NaiveDate;

use super::super::DateFlags;
use super::resolve::{self, Fields, Label};
use super::tokens::{MONTHS, Token, WEEKDAYS, is_jump, lookup};

pub(super) struct Parser<'a> {
    pub(super) tokens: &'a [Token],
    pub(super) today: NaiveDate,
    pub(super) flags: DateFlags,
    pub(super) fields: Fields,
    pub(super) weekday: Option<i64>,
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
        let token = self.tokens.get(index)?.clone();
        if let Token::Number(_) = token {
            if self.starts_time(index) {
                return Some(self.skip_time(index));
            }
            return self.number_token(index);
        }
        if matches!(token, Token::Punct('+' | '-')) {
            return Some(self.skip_offset(index));
        }
        if let Token::Word(_) = token {
            return self.word_token(index);
        }
        Some(index)
    }

    // the clock time only says how far into the day the moment is, and the caller reads the day
    fn skip_time(&mut self, index: usize) -> usize {
        self.saw = true;
        let mut last = index.saturating_add(2);
        if self.tokens.get(index.saturating_add(3)) == Some(&Token::Punct(':')) {
            last = index.saturating_add(4);
        }
        // a fraction of a second trails the last member as ".5"
        if self.tokens.get(last.saturating_add(1)) == Some(&Token::Punct('.'))
            && matches!(
                self.tokens.get(last.saturating_add(2)),
                Some(Token::Number(_))
            )
        {
            last = last.saturating_add(2);
        }
        last
    }

    // a time zone offset after a time, as "+0000" or "+05:00"
    fn skip_offset(&self, index: usize) -> usize {
        let next = index.saturating_add(1);
        match self.tokens.get(next) {
            Some(Token::Number(text)) if text.len() == 4 => next,
            Some(Token::Number(text))
                if text.len() <= 2
                    && self.tokens.get(next.saturating_add(1)) == Some(&Token::Punct(':')) =>
            {
                next.saturating_add(2)
            }
            _ => index,
        }
    }

    fn word_token(&mut self, index: usize) -> Option<usize> {
        let token = self.tokens.get(index)?.clone();
        if let Some(month) = lookup(&token, &MONTHS).map(|index| index.saturating_add(1)) {
            self.fields
                .push_text(&month.to_string(), Some(Label::Month))?;
            return self.month_member(index);
        }
        if let Some(weekday) = lookup(&token, &WEEKDAYS) {
            self.weekday = Some(weekday);
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
                let month = lookup(token, &MONTHS).map(|index| index.saturating_add(1))?;
                self.fields
                    .push_text(&month.to_string(), Some(Label::Month))
            }
            Token::Punct(_) => None,
        }
    }

    fn number_token(&mut self, index: usize) -> Option<usize> {
        let Token::Number(text) = self.tokens.get(index)? else {
            return None;
        };
        let members = self.fields.items.len();
        match (text.len(), members) {
            (6, 0) => self.push_compact(text, 2)?,
            (6, _) | (2 | 4, 3) => return Some(index),
            (8 | 12 | 14, _) => self.push_compact(text, 4)?,
            _ => return self.number_tail(index, text.parse().ok()?),
        }
        Some(index)
    }

    // a compact run is YYMMDD, or YYYYMMDD with its clock time dropped
    fn push_compact(&mut self, text: &str, width: usize) -> Option<()> {
        let (middle, end) = (width.checked_add(2)?, width.checked_add(4)?);
        let label = (width == 4).then_some(Label::Year);
        self.fields.push_text(text.get(..width)?, label)?;
        self.fields.push_text(text.get(width..middle)?, None)?;
        self.fields.push_text(text.get(middle..end)?, None)
    }

    fn number_tail(&mut self, index: usize, value: i64) -> Option<usize> {
        let following = self.tokens.get(index.saturating_add(1));
        if following.is_some_and(|token| matches!(token, Token::Punct('-' | '/' | '.'))) {
            return self.separator_group(index);
        }
        // a clock time after this number is read on its own, so this one closes the date
        let closes = following.is_none_or(is_jump) || self.starts_time(index.saturating_add(1));
        if closes || self.could_be_day(value)? {
            let long = value > 100;
            self.fields.push(value, None, long)?;
        }
        Some(index)
    }

    fn starts_time(&self, index: usize) -> bool {
        matches!(self.tokens.get(index), Some(Token::Number(_)))
            && self.tokens.get(index.saturating_add(1)) == Some(&Token::Punct(':'))
    }

    fn separator_group(&mut self, index: usize) -> Option<usize> {
        let tokens = self.tokens;
        let separator = tokens.get(index.saturating_add(1))?.clone();
        let Some(Token::Number(text)) = tokens.get(index) else {
            return None;
        };
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
}
