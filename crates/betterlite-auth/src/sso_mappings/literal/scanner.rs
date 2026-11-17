use super::{Cursor, LiteralError, MAX_LITERAL_DEPTH};

impl Cursor<'_> {
    // outside brackets a newline ends the expression, so a value may not continue on the next line
    pub(super) fn skip_trivia(&mut self, depth: usize) -> Result<bool, LiteralError> {
        let mut crossed_line = false;
        loop {
            match self.byte() {
                Some(b' ' | b'\t' | b'\x0c') => self.at = self.at.saturating_add(1),
                Some(b'\n' | b'\r') => {
                    crossed_line = crossed_line || depth == 0;
                    self.at = self.at.saturating_add(1);
                }
                Some(b'#') => self.skip_comment(),
                Some(b'\\') => self.skip_continuation()?,
                _other => return Ok(crossed_line),
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(byte) = self.byte() {
            if matches!(byte, b'\n' | b'\r') {
                return;
            }
            self.at = self.at.saturating_add(1);
        }
    }

    fn skip_continuation(&mut self) -> Result<(), LiteralError> {
        let Some(rest) = self.text.get(self.at.saturating_add(1)..) else {
            return Err(LiteralError::Syntax);
        };
        let Some(next) = rest.chars().next() else {
            return Err(LiteralError::Syntax);
        };
        match next {
            '\n' => {
                self.at = self.at.saturating_add(2);
                Ok(())
            }
            '\r' => {
                self.at = self.skip_newline_of(self.at.saturating_add(2));
                Ok(())
            }
            _other => Err(LiteralError::Syntax),
        }
    }

    pub(super) fn skip_newline_of(&self, at: usize) -> usize {
        if self.bytes.get(at).copied() == Some(b'\n') {
            return at.saturating_add(1);
        }
        at
    }

    pub(super) fn open(&mut self, depth: usize) -> Result<(), LiteralError> {
        if depth >= MAX_LITERAL_DEPTH {
            return Err(LiteralError::TooDeep {
                limit: MAX_LITERAL_DEPTH,
            });
        }
        self.at = self.at.saturating_add(1);
        self.skip_trivia(depth.saturating_add(1))?;
        Ok(())
    }

    pub(super) fn close_or_comma(&mut self, depth: usize, close: u8) -> Result<bool, LiteralError> {
        self.skip_trivia(depth.saturating_add(1))?;
        let Some(byte) = self.byte() else {
            return Err(LiteralError::Syntax);
        };
        if byte == close {
            self.at = self.at.saturating_add(1);
            return Ok(false);
        }
        if byte != b',' {
            return Err(LiteralError::Syntax);
        }
        self.at = self.at.saturating_add(1);
        self.skip_trivia(depth.saturating_add(1))?;
        if self.byte() == Some(close) {
            self.at = self.at.saturating_add(1);
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) fn character(&self, at: usize) -> Option<char> {
        let Some(rest) = self.text.get(at..) else {
            return None;
        };
        rest.chars().next()
    }
}
