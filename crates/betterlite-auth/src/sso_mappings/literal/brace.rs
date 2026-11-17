use super::{Cursor, Literal, LiteralError};

impl Cursor<'_> {
    pub(super) fn parse_brace(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        if self.open(depth, b'}')? {
            return Ok(Literal::Dict(Vec::new()));
        }
        self.parse_brace_body(depth)
    }

    fn parse_brace_body(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        let first = self.first_element(depth)?;
        if self.bytes.get(self.at).copied() == Some(b':') {
            return self.parse_dict(depth, first);
        }
        self.parse_rest(depth, b'}', vec![first]).map(Literal::Set)
    }

    fn parse_dict(&mut self, depth: usize, first: Literal) -> Result<Literal, LiteralError> {
        let mut entries = Vec::new();
        let mut key = first;
        loop {
            self.at = self.at.saturating_add(1); // the colon after the key
            self.skip_trivia(depth.saturating_add(1))?;
            let value = self.parse_value(depth.saturating_add(1))?;
            replace_entry(&mut entries, key, value);
            if !self.close_or_comma(depth, b'}')? {
                return Ok(Literal::Dict(entries));
            }
            key = self.parse_next_key(depth)?;
        }
    }

    fn parse_next_key(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        let key = self.parse_value(depth.saturating_add(1))?;
        self.expect(depth, b':')?;
        Ok(key)
    }

    fn expect(&mut self, depth: usize, expected: u8) -> Result<(), LiteralError> {
        self.skip_trivia(depth.saturating_add(1))?;
        if self.bytes.get(self.at).copied() != Some(expected) {
            return Err(LiteralError::Syntax);
        }
        Ok(())
    }
}

// a repeated text key keeps its first position and its last value, as Python's dict does
fn replace_entry(entries: &mut Vec<(Literal, Literal)>, key: Literal, value: Literal) {
    let Literal::Str(text) = &key else {
        entries.push((key, value));
        return;
    };
    let duplicate = entries.iter_mut().find(|entry| match &entry.0 {
        Literal::Str(existing) => existing == text,
        _other => false,
    });
    match duplicate {
        Some(entry) => entry.1 = value,
        None => entries.push((key, value)),
    }
}
