use super::super::PatternProblem;
use super::{ClassEntry, PrevKind, Scanner, SeqState};

impl Scanner {
    pub(super) fn scan_class(
        &mut self,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        let mut items: Vec<ClassEntry> = Vec::new();
        let mut negated = false;
        if self.peek() == Some('^') {
            self.take();
            negated = true;
        }
        loop {
            let Some(character) = self.take() else {
                return Err(PatternProblem::Syntax);
            };
            if character == ']' && !items.is_empty() {
                break;
            }
            let entry = if character == '\\' {
                self.class_escape()?
            } else {
                ClassEntry::Literal(u32::from(character))
            };
            if self.peek() == Some('-') {
                self.take();
                let Some(next) = self.take() else {
                    return Err(PatternProblem::Syntax);
                };
                if next == ']' {
                    items.push(entry);
                    items.push(ClassEntry::Literal(u32::from('-')));
                    break;
                }
                let second = if next == '\\' {
                    self.class_escape()?
                } else {
                    ClassEntry::Literal(u32::from(next))
                };
                let (ClassEntry::Literal(low), ClassEntry::Literal(high)) = (entry, second) else {
                    return Err(PatternProblem::Syntax);
                };
                if low > high {
                    return Err(PatternProblem::Syntax);
                }
                items.push(ClassEntry::Range(low, high));
            } else {
                items.push(entry);
            }
        }
        if items
            .iter()
            .any(|item| matches!(item, ClassEntry::Unsupported))
        {
            self.replace_with_marker(start, state, PrevKind::Other);
            return Ok(());
        }
        self.output.push('[');
        if negated {
            self.output.push('^');
        }
        for item in items {
            match item {
                ClassEntry::Literal(code) => self.push_hex(code),
                ClassEntry::Range(low, high) => {
                    self.push_hex(low);
                    self.output.push('-');
                    self.push_hex(high);
                }
                ClassEntry::Category { negated, kind } => {
                    self.output.push('\\');
                    let emitted = if negated {
                        kind.to_ascii_uppercase()
                    } else {
                        kind
                    };
                    self.output.push(emitted);
                }
                ClassEntry::Unsupported => {}
            }
        }
        self.output.push(']');
        state.node(PrevKind::Other, start);
        Ok(())
    }
}
