use super::super::PatternProblem;
use super::{PrevKind, Scanner, SeqState};

const MAX_CODE_POINT: u32 = 0x10_F_FFFF;
const SURROGATE_START: u32 = 0xD800;
const SURROGATE_END: u32 = 0xDFFF;

impl Scanner {
    pub(super) fn scan_escape(&mut self, state: &mut SeqState) -> Result<(), PatternProblem> {
        let start = self.output.len();
        self.take();
        let Some(character) = self.take() else {
            return Err(PatternProblem::Syntax);
        };
        match character {
            'A' => self.assertion_text("\\A", PrevKind::Assertion, state, start),
            'Z' => self.assertion_text("\\z", PrevKind::Assertion, state, start),
            'b' => self.assertion_text("\\b", PrevKind::Assertion, state, start),
            'B' => self.assertion_text("\\B", PrevKind::Assertion, state, start),
            'd' | 'D' | 's' | 'S' | 'w' | 'W' => {
                self.output.push('\\');
                self.output.push(character);
                state.node(PrevKind::Other, start);
                Ok(())
            }
            'a' => self.literal_code(0x07, state, start),
            'f' => self.literal_code(0x0C, state, start),
            'n' => self.literal_code(0x0A, state, start),
            'r' => self.literal_code(0x0D, state, start),
            't' => self.literal_code(0x09, state, start),
            'v' => self.literal_code(0x0B, state, start),
            '\\' => self.literal_code(0x5C, state, start),
            'x' => {
                let value = self.read_hex(2)?;
                self.literal_code(value, state, start)
            }
            'u' => {
                let value = self.read_hex(4)?;
                self.code_point(value, state, start)
            }
            'U' => {
                let value = self.read_hex(8)?;
                if value > MAX_CODE_POINT {
                    return Err(PatternProblem::Syntax);
                }
                self.code_point(value, state, start)
            }
            'N' => self.scan_named(state, start),
            '0' => {
                let value = self.read_octal_more(u32::from(character.to_digit(8).unwrap_or(0)))?;
                self.literal_code(value, state, start)
            }
            '1'..='9' => self.scan_group_reference(character, state, start),
            other if other.is_ascii_alphabetic() => Err(PatternProblem::Syntax),
            other => self.literal_code(u32::from(other), state, start),
        }
    }

    fn assertion_text(
        &mut self,
        text: &str,
        kind: PrevKind,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        self.output.push_str(text);
        state.node(kind, start);
        Ok(())
    }

    fn literal_code(
        &mut self,
        code: u32,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        self.push_hex(code);
        state.node(PrevKind::Other, start);
        Ok(())
    }

    fn code_point(
        &mut self,
        code: u32,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        if (SURROGATE_START..=SURROGATE_END).contains(&code) {
            self.replace_with_marker(start, state, PrevKind::Other);
            return Ok(());
        }
        self.literal_code(code, state, start)
    }

    fn scan_named(&mut self, state: &mut SeqState, start: usize) -> Result<(), PatternProblem> {
        if self.take() != Some('{') {
            return Err(PatternProblem::Syntax);
        }
        let _name = self.take_until('}')?;
        self.replace_with_marker(start, state, PrevKind::Other);
        Ok(())
    }

    fn scan_group_reference(
        &mut self,
        first: char,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        let first_value = u32::from(first.to_digit(10).unwrap_or(0));
        let mut value = u64::from(first_value);
        let first_two_octal = first <= '7';
        if let Some(second) = self.peek()
            && second.is_ascii_digit()
        {
            self.take();
            let second_value = u32::from(second.to_digit(10).unwrap_or(0));
            value = value
                .saturating_mul(10)
                .saturating_add(u64::from(second_value));
            if first_two_octal && second <= '7' {
                if let Some(third) = self.peek()
                    && matches!(third, '0'..='7')
                {
                    self.take();
                    let third_value = u32::from(third.to_digit(8).unwrap_or(0));
                    let octal = first_value
                        .saturating_mul(64)
                        .saturating_add(second_value.saturating_mul(8))
                        .saturating_add(third_value);
                    if octal > 0o377 {
                        return Err(PatternProblem::Syntax);
                    }
                    self.literal_code(octal, state, start)?;
                    return Ok(());
                }
            }
        }
        self.check_reference(value)?;
        self.replace_with_marker(start, state, PrevKind::Other);
        Ok(())
    }

    pub(super) fn read_hex(&mut self, count: u32) -> Result<u32, PatternProblem> {
        let mut value = 0_u32;
        for _ in 0..count {
            let Some(character) = self.take() else {
                return Err(PatternProblem::Syntax);
            };
            let Some(digit) = character.to_digit(16) else {
                return Err(PatternProblem::Syntax);
            };
            value = value.saturating_mul(16).saturating_add(digit);
        }
        Ok(value)
    }

    pub(super) fn read_octal_more(&mut self, first: u32) -> Result<u32, PatternProblem> {
        let mut value = first;
        for _ in 0..2 {
            let Some(character) = self.peek() else {
                break;
            };
            if !matches!(character, '0'..='7') {
                break;
            }
            self.take();
            value = value
                .saturating_mul(8)
                .saturating_add(u32::from(character.to_digit(8).unwrap_or(0)));
        }
        Ok(value)
    }

    pub(super) fn class_escape(&mut self) -> Result<super::ClassEntry, PatternProblem> {
        let Some(character) = self.take() else {
            return Err(PatternProblem::Syntax);
        };
        match character {
            'a' => Ok(super::ClassEntry::Literal(0x07)),
            'b' => Ok(super::ClassEntry::Literal(0x08)),
            'f' => Ok(super::ClassEntry::Literal(0x0C)),
            'n' => Ok(super::ClassEntry::Literal(0x0A)),
            'r' => Ok(super::ClassEntry::Literal(0x0D)),
            't' => Ok(super::ClassEntry::Literal(0x09)),
            'v' => Ok(super::ClassEntry::Literal(0x0B)),
            '\\' => Ok(super::ClassEntry::Literal(0x5C)),
            'd' | 's' | 'w' => Ok(super::ClassEntry::Category {
                negated: false,
                kind: character,
            }),
            'D' | 'S' | 'W' => Ok(super::ClassEntry::Category {
                negated: true,
                kind: character.to_ascii_lowercase(),
            }),
            'x' => Ok(super::ClassEntry::Literal(self.read_hex(2)?)),
            'u' => self.class_code(self.read_hex(4)?),
            'U' => {
                let value = self.read_hex(8)?;
                if value > MAX_CODE_POINT {
                    return Err(PatternProblem::Syntax);
                }
                self.class_code(value)
            }
            'N' => {
                if self.take() != Some('{') {
                    return Err(PatternProblem::Syntax);
                }
                let _name = self.take_until('}')?;
                Ok(super::ClassEntry::Unsupported)
            }
            '0'..='7' => {
                let first = u32::from(character.to_digit(8).unwrap_or(0));
                let value = self.read_octal_more(first)?;
                if value > 0o377 {
                    return Err(PatternProblem::Syntax);
                }
                Ok(super::ClassEntry::Literal(value))
            }
            other if other.is_ascii_alphabetic() => Err(PatternProblem::Syntax),
            other => Ok(super::ClassEntry::Literal(u32::from(other))),
        }
    }

    fn class_code(&self, code: u32) -> Result<super::ClassEntry, PatternProblem> {
        if (SURROGATE_START..=SURROGATE_END).contains(&code) {
            return Ok(super::ClassEntry::Unsupported);
        }
        Ok(super::ClassEntry::Literal(code))
    }
}
