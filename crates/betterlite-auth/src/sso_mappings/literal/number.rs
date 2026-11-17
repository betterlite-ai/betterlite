use super::{Cursor, Literal, LiteralError};

struct Digits {
    count: usize,
    nonzero: bool,
}

impl Cursor<'_> {
    pub(super) fn number_start(&self) -> bool {
        match self.byte() {
            Some(byte) if byte.is_ascii_digit() => true,
            Some(b'.') => {
                let Some(next) = self.bytes.get(self.at.saturating_add(1)).copied() else {
                    return false;
                };
                next.is_ascii_digit()
            }
            _other => false,
        }
    }

    pub(super) fn parse_number(&mut self) -> Result<Literal, LiteralError> {
        let Some(first) = self.byte() else {
            return Err(LiteralError::Syntax);
        };
        if first == b'0' {
            if let Some(radix) = self.radix() {
                self.scan_radix(radix)?;
                return Ok(Literal::Number);
            }
        }
        self.scan_decimal()?;
        self.skip_imaginary();
        Ok(Literal::Number)
    }

    fn radix(&self) -> Option<u32> {
        let lowered = self.bytes.get(self.at.saturating_add(1))?;
        match lowered {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _other => None,
        }
    }

    fn scan_radix(&mut self, radix: u32) -> Result<(), LiteralError> {
        self.at = self.at.saturating_add(2);
        let mut digits = 0_usize;
        let mut after_underscore = false;
        loop {
            let Some(byte) = self.byte() else {
                break;
            };
            if byte == b'_' {
                if after_underscore {
                    return Err(LiteralError::Syntax);
                }
                after_underscore = true;
                self.at = self.at.saturating_add(1);
                continue;
            }
            if !char::from(byte).is_digit(radix) {
                break;
            }
            digits = digits.saturating_add(1);
            after_underscore = false;
            self.at = self.at.saturating_add(1);
        }
        if digits == 0 || after_underscore {
            return Err(LiteralError::Syntax);
        }
        Ok(())
    }

    fn scan_decimal(&mut self) -> Result<(), LiteralError> {
        let start = self.at;
        let integer = self.scan_digits()?;
        let mut is_float = false;
        if self.byte() == Some(b'.') {
            is_float = true;
            self.at = self.at.saturating_add(1);
            let fraction = self.scan_digits()?;
            if integer.count == 0 && fraction.count == 0 {
                return Err(LiteralError::Syntax);
            }
        }
        if matches!(self.byte(), Some(b'e' | b'E')) {
            is_float = true;
            self.at = self.at.saturating_add(1);
            if matches!(self.byte(), Some(b'+' | b'-')) {
                self.at = self.at.saturating_add(1);
            }
            let exponent = self.scan_digits()?;
            if exponent.count == 0 {
                return Err(LiteralError::Syntax);
            }
        }
        if is_float {
            return Ok(());
        }
        let leading_zero = self.bytes.get(start).copied() == Some(b'0');
        if leading_zero && integer.nonzero {
            return Err(LiteralError::Syntax);
        }
        Ok(())
    }

    fn scan_digits(&mut self) -> Result<Digits, LiteralError> {
        let mut digits = Digits {
            count: 0,
            nonzero: false,
        };
        let mut after_underscore = false;
        while let Some(byte) = self.byte() {
            if byte.is_ascii_digit() {
                if byte != b'0' {
                    digits.nonzero = true;
                }
                digits.count = digits.count.saturating_add(1);
                after_underscore = false;
                self.at = self.at.saturating_add(1);
                continue;
            }
            if byte != b'_' {
                break;
            }
            if after_underscore || digits.count == 0 {
                return Err(LiteralError::Syntax);
            }
            after_underscore = true;
            self.at = self.at.saturating_add(1);
        }
        if after_underscore {
            return Err(LiteralError::Syntax);
        }
        Ok(digits)
    }

    fn skip_imaginary(&mut self) {
        if matches!(self.byte(), Some(b'j' | b'J')) {
            self.at = self.at.saturating_add(1);
        }
    }
}
