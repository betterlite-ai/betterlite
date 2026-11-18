use super::{MAX_REPEAT, PatternProblem};

mod class;
mod escape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevKind {
    Other,
    Assertion,
    Repeat,
}

#[derive(Debug, Clone, Copy, Default)]
struct SeqState {
    last: Option<PrevKind>,
    start: usize,
}

impl SeqState {
    fn node(&mut self, kind: PrevKind, start: usize) {
        self.last = Some(kind);
        self.start = start;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClassEntry {
    Literal(u32),
    Range(u32, u32),
    Category { negated: bool, kind: char },
    Unsupported,
}

#[derive(Default)]
struct FlagAdd {
    ignore_case: bool,
    multi_line: bool,
    dot_all: bool,
    verbose: bool,
    ascii: bool,
    unicode: bool,
}

#[derive(Default)]
struct FlagDel {
    ignore_case: bool,
    multi_line: bool,
    dot_all: bool,
    verbose: bool,
}

struct Scanner {
    input: Vec<char>,
    index: usize,
    output: String,
    verbose: bool,
    global_ascii: bool,
    global_unicode: bool,
    opened: u64,
    closed: Vec<bool>,
    names: Vec<(String, u64)>,
    next_name: u64,
    lookbehind: Option<u64>,
    pending_conditions: Vec<u64>,
}

pub(super) fn canonical(pattern: &str) -> Result<String, PatternProblem> {
    let mut scanner = Scanner::new(pattern);
    let _state = scanner.scan_sequence(true)?;
    if scanner.global_ascii && scanner.global_unicode {
        return Err(PatternProblem::Syntax);
    }
    if scanner
        .pending_conditions
        .iter()
        .any(|id| *id > scanner.opened)
    {
        return Err(PatternProblem::Syntax);
    }
    Ok(scanner.output)
}

impl Scanner {
    fn new(pattern: &str) -> Self {
        Self {
            input: pattern.chars().collect(),
            index: 0,
            output: String::new(),
            verbose: false,
            global_ascii: false,
            global_unicode: false,
            opened: 0,
            closed: Vec::new(),
            names: Vec::new(),
            next_name: 0,
            lookbehind: None,
            pending_conditions: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.index).copied()
    }

    fn take(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.index = self.index.saturating_add(1);
        Some(character)
    }

    fn scan_sequence(&mut self, top: bool) -> Result<SeqState, PatternProblem> {
        let mut state = SeqState::default();
        let mut branches = 0_usize;
        while let Some(character) = self.peek() {
            match character {
                ')' => {
                    if top {
                        return Err(PatternProblem::Syntax);
                    }
                    return Ok(state);
                }
                '|' => {
                    self.take();
                    self.output.push('|');
                    state = SeqState::default();
                    branches = branches.saturating_add(1);
                }
                '\\' => self.scan_escape(&mut state)?,
                '#' if self.verbose => self.skip_comment_line(),
                _ if self.verbose && is_verbose_space(character) => {
                    self.take();
                }
                '*' | '+' | '?' | '{' => self.scan_quantifier(character, &mut state)?,
                _ => {
                    let start = self.output.len();
                    self.take();
                    self.scan_character(character, top && branches == 0, start, &mut state)?;
                }
            }
        }
        Ok(state)
    }

    fn scan_character(
        &mut self,
        character: char,
        first: bool,
        start: usize,
        state: &mut SeqState,
    ) -> Result<(), PatternProblem> {
        match character {
            '(' => self.scan_group(state, first, start),
            '[' => self.scan_class(state, start),
            '^' => {
                self.output.push('^');
                state.node(PrevKind::Assertion, start);
                Ok(())
            }
            '$' => {
                self.output.push('$');
                state.node(PrevKind::Assertion, start);
                Ok(())
            }
            '.' => {
                self.output.push('.');
                state.node(PrevKind::Other, start);
                Ok(())
            }
            other => {
                self.push_hex(u32::from(other));
                state.node(PrevKind::Other, start);
                Ok(())
            }
        }
    }

    fn scan_quantifier(
        &mut self,
        character: char,
        state: &mut SeqState,
    ) -> Result<(), PatternProblem> {
        if character == '{' {
            return self.scan_counted(state);
        }
        self.take();
        check_repeat_target(state)?;
        self.output.push(character);
        self.finish_quantifier(state)
    }

    fn scan_counted(&mut self, state: &mut SeqState) -> Result<(), PatternProblem> {
        let start = self.output.len();
        self.take();
        if self.peek() == Some('}') {
            self.literal_brace(start, state);
            return Ok(());
        }
        let after_brace = self.index;
        let low = self.take_decimal();
        let comma = self.peek() == Some(',');
        if comma {
            self.take();
        }
        let high = if comma {
            self.take_decimal()
        } else {
            String::new()
        };
        if self.peek() != Some('}') {
            self.index = after_brace;
            self.literal_brace(start, state);
            return Ok(());
        }
        self.take();
        let min = if low.is_empty() {
            0
        } else {
            count_value(&low)?
        };
        let max = if comma {
            if high.is_empty() {
                MAX_REPEAT
            } else {
                count_value(&high)?
            }
        } else {
            min
        };
        if comma && !high.is_empty() && max < min {
            return Err(PatternProblem::Syntax);
        }
        check_repeat_target(state)?;
        self.push_counted(min, max);
        self.finish_quantifier(state)
    }

    fn literal_brace(&mut self, start: usize, state: &mut SeqState) {
        self.push_hex(u32::from('{'));
        state.node(PrevKind::Other, start);
    }

    fn finish_quantifier(&mut self, state: &mut SeqState) -> Result<(), PatternProblem> {
        if self.peek() == Some('?') {
            self.take();
            self.output.push('?');
            state.last = Some(PrevKind::Repeat);
            return Ok(());
        }
        if self.peek() == Some('+') {
            self.take();
            let start = state.start;
            self.replace_with_marker(start, state, PrevKind::Repeat);
            state.last = Some(PrevKind::Repeat);
            return Ok(());
        }
        state.last = Some(PrevKind::Repeat);
        Ok(())
    }

    fn push_counted(&mut self, min: u64, max: u64) {
        if min == 0 && max == MAX_REPEAT {
            self.output.push('*');
        } else if min == 1 && max == MAX_REPEAT {
            self.output.push('+');
        } else if min == 0 && max == 1 {
            self.output.push('?');
        } else {
            self.output.push('{');
            self.push_decimal(min);
            if max == MAX_REPEAT {
                self.output.push(',');
            } else if max != min {
                self.output.push(',');
                self.push_decimal(max);
            }
            self.output.push('}');
        }
    }

    fn take_decimal(&mut self) -> String {
        let mut digits = String::new();
        while let Some(character) = self.peek() {
            if !character.is_ascii_digit() {
                break;
            }
            self.take();
            digits.push(character);
        }
        digits
    }

    fn scan_group(
        &mut self,
        state: &mut SeqState,
        first: bool,
        start: usize,
    ) -> Result<(), PatternProblem> {
        if self.peek() != Some('?') {
            let id = self.open_capture(None)?;
            self.emit_capture(id);
            self.scan_sequence(false)?;
            self.expect_close()?;
            self.close_capture(id);
            self.output.push(')');
            state.node(PrevKind::Other, start);
            return Ok(());
        }
        self.take();
        let Some(character) = self.take() else {
            return Err(PatternProblem::Syntax);
        };
        match character {
            ':' => {
                self.output.push_str("(?:");
                self.scan_sequence(false)?;
                self.expect_close()?;
                self.output.push(')');
                state.node(PrevKind::Other, start);
                Ok(())
            }
            '#' => self.skip_comment_group(),
            'P' => self.scan_python_group(state, start),
            '=' | '!' => self.scan_lookaround(state, start, false),
            '<' => match self.take() {
                Some('=' | '!') => self.scan_lookaround(state, start, true),
                _ => Err(PatternProblem::Syntax),
            },
            '>' => self.scan_atomic(state, start),
            '(' => self.scan_conditional(state, start),
            flag if is_flag(flag) || flag == '-' => {
                self.scan_flags(flag, first && state.last.is_none(), state, start)
            }
            _ => Err(PatternProblem::Syntax),
        }
    }

    fn expect_close(&mut self) -> Result<(), PatternProblem> {
        if self.take() == Some(')') {
            Ok(())
        } else {
            Err(PatternProblem::Syntax)
        }
    }

    fn skip_comment_line(&mut self) {
        while let Some(character) = self.take() {
            if character == '\n' {
                return;
            }
        }
    }

    fn skip_comment_group(&mut self) -> Result<(), PatternProblem> {
        loop {
            match self.take() {
                None => return Err(PatternProblem::Syntax),
                Some(')') => return Ok(()),
                Some('\\') => {
                    if self.take().is_none() {
                        return Err(PatternProblem::Syntax);
                    }
                }
                Some(_) => {}
            }
        }
    }

    fn scan_python_group(
        &mut self,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        match self.take() {
            Some('<') => {
                let name = self.take_until('>')?;
                self.check_identifier(&name)?;
                let id = self.open_capture(Some(name))?;
                self.emit_capture(id);
                self.scan_sequence(false)?;
                self.expect_close()?;
                self.close_capture(id);
                self.output.push(')');
                state.node(PrevKind::Other, start);
                Ok(())
            }
            Some('=') => {
                let name = self.take_until(')')?;
                let id = self.lookup_name(&name)?;
                self.check_reference(id)?;
                self.replace_with_marker(start, state, PrevKind::Other);
                Ok(())
            }
            _ => Err(PatternProblem::Syntax),
        }
    }

    fn scan_lookaround(
        &mut self,
        state: &mut SeqState,
        start: usize,
        behind: bool,
    ) -> Result<(), PatternProblem> {
        let previous = self.lookbehind;
        if behind {
            self.lookbehind = Some(self.opened.saturating_add(1));
        }
        let body = self.scan_sequence(false);
        self.lookbehind = previous;
        body?;
        self.expect_close()?;
        self.replace_with_marker(start, state, PrevKind::Other);
        Ok(())
    }

    fn scan_atomic(&mut self, state: &mut SeqState, start: usize) -> Result<(), PatternProblem> {
        self.scan_sequence(false)?;
        self.expect_close()?;
        self.replace_with_marker(start, state, PrevKind::Other);
        Ok(())
    }

    fn scan_conditional(
        &mut self,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        let condition = self.take_until(')')?;
        if condition
            .chars()
            .all(|character| character.is_ascii_digit())
        {
            if let Ok(id) = condition.parse::<u64>() {
                if id == 0 {
                    return Err(PatternProblem::Syntax);
                }
                if let Some(base) = self.lookbehind {
                    if id >= base {
                        return Err(PatternProblem::Syntax);
                    }
                }
                self.pending_conditions.push(id);
            }
        } else if self.lookup_name(&condition).is_err() {
            return Err(PatternProblem::Syntax);
        }
        self.scan_sequence(false)?;
        self.expect_close()?;
        self.replace_with_marker(start, state, PrevKind::Other);
        Ok(())
    }

    fn scan_flags(
        &mut self,
        first: char,
        allow_global: bool,
        state: &mut SeqState,
        start: usize,
    ) -> Result<(), PatternProblem> {
        let mut add = FlagAdd::default();
        let mut del = FlagDel::default();
        let mut global = false;
        if first == '-' {
            let Some(flag) = self.take() else {
                return Err(PatternProblem::Syntax);
            };
            add_del_flag(&mut del, flag)?;
            loop {
                let Some(character) = self.take() else {
                    return Err(PatternProblem::Syntax);
                };
                if character == ':' {
                    break;
                }
                add_del_flag(&mut del, character)?;
            }
        } else {
            add_add_flag(&mut add, first)?;
            loop {
                let Some(character) = self.take() else {
                    return Err(PatternProblem::Syntax);
                };
                match character {
                    ')' => {
                        global = true;
                        break;
                    }
                    ':' => break,
                    '-' => {
                        let Some(flag) = self.take() else {
                            return Err(PatternProblem::Syntax);
                        };
                        add_del_flag(&mut del, flag)?;
                        loop {
                            let Some(character) = self.take() else {
                                return Err(PatternProblem::Syntax);
                            };
                            if character == ':' {
                                break;
                            }
                            add_del_flag(&mut del, character)?;
                        }
                        break;
                    }
                    _ => add_add_flag(&mut add, character)?,
                }
            }
        }
        if global {
            if !allow_global {
                return Err(PatternProblem::Syntax);
            }
            self.verbose = self.verbose || add.verbose;
            self.global_ascii = self.global_ascii || add.ascii;
            self.global_unicode = self.global_unicode || add.unicode;
            return Ok(());
        }
        if add.ignore_case && del.ignore_case
            || add.multi_line && del.multi_line
            || add.dot_all && del.dot_all
            || add.verbose && del.verbose
        {
            return Err(PatternProblem::Syntax);
        }
        self.emit_flags(&add, &del);
        let previous = self.verbose;
        self.verbose = (previous || add.verbose) && !del.verbose;
        let body = self.scan_sequence(false);
        self.verbose = previous;
        body?;
        self.expect_close()?;
        self.output.push(')');
        state.node(PrevKind::Other, start);
        Ok(())
    }

    fn emit_flags(&mut self, add: &FlagAdd, del: &FlagDel) {
        self.output.push_str("(?");
        if add.ascii || add.unicode {
            self.output.push('u');
        }
        if add.ignore_case {
            self.output.push('i');
        }
        if add.multi_line {
            self.output.push('m');
        }
        if add.dot_all {
            self.output.push('s');
        }
        if add.verbose {
            self.output.push('x');
        }
        if del.ignore_case || del.multi_line || del.dot_all || del.verbose {
            self.output.push('-');
            if del.ignore_case {
                self.output.push('i');
            }
            if del.multi_line {
                self.output.push('m');
            }
            if del.dot_all {
                self.output.push('s');
            }
            if del.verbose {
                self.output.push('x');
            }
        }
        self.output.push(':');
    }

    fn open_capture(&mut self, name: Option<String>) -> Result<u64, PatternProblem> {
        self.opened = self.opened.saturating_add(1);
        let id = self.opened;
        self.closed.push(false);
        if let Some(name) = name {
            if self.names.iter().any(|(existing, _)| *existing == name) {
                return Err(PatternProblem::Syntax);
            }
            self.names.push((name, id));
        }
        Ok(id)
    }

    fn close_capture(&mut self, id: u64) {
        let index = usize::try_from(id.saturating_sub(1)).unwrap_or(0);
        if let Some(slot) = self.closed.get_mut(index) {
            *slot = true;
        }
    }

    fn lookup_name(&self, name: &str) -> Result<u64, PatternProblem> {
        self.names
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, id)| *id)
            .ok_or(PatternProblem::Syntax)
    }

    fn check_reference(&self, id: u64) -> Result<(), PatternProblem> {
        if id == 0 || id > self.opened {
            return Err(PatternProblem::Syntax);
        }
        let index = usize::try_from(id.saturating_sub(1)).unwrap_or(usize::MAX);
        if self.closed.get(index).copied() != Some(true) {
            return Err(PatternProblem::Syntax);
        }
        if let Some(base) = self.lookbehind
            && id >= base
        {
            return Err(PatternProblem::Syntax);
        }
        Ok(())
    }

    fn check_identifier(&self, name: &str) -> Result<(), PatternProblem> {
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return Err(PatternProblem::Syntax);
        };
        if first != '_' && !unicode_ident::is_xid_start(first) {
            return Err(PatternProblem::Syntax);
        }
        if characters.all(unicode_ident::is_xid_continue) {
            Ok(())
        } else {
            Err(PatternProblem::Syntax)
        }
    }

    fn take_until(&mut self, terminator: char) -> Result<String, PatternProblem> {
        let mut text = String::new();
        loop {
            match self.take() {
                None => return Err(PatternProblem::Syntax),
                Some(character) if character == terminator => {
                    if text.is_empty() {
                        return Err(PatternProblem::Syntax);
                    }
                    return Ok(text);
                }
                Some('\\') => {
                    let Some(escaped) = self.take() else {
                        return Err(PatternProblem::Syntax);
                    };
                    text.push('\\');
                    text.push(escaped);
                }
                Some(character) => text.push(character),
            }
        }
    }

    fn emit_capture(&mut self, id: u64) {
        self.output.push_str("(?P<g");
        self.push_decimal(id);
        self.output.push('>');
    }

    fn marker(&mut self) {
        self.output.push_str("(?P<u");
        self.push_decimal(self.next_name);
        self.output.push_str(">)");
        self.next_name = self.next_name.saturating_add(1);
    }

    fn replace_with_marker(&mut self, start: usize, state: &mut SeqState, kind: PrevKind) {
        self.output.truncate(start);
        self.marker();
        state.node(kind, start);
    }

    fn push_hex(&mut self, code: u32) {
        self.output.push_str("\\x{");
        for shift in [20, 16, 12, 8, 4, 0] {
            let digit = code.checked_shr(shift).unwrap_or(0) & 0xF;
            if let Some(character) = char::from_digit(digit, 16) {
                self.output.push(character);
            }
        }
        self.output.push('}');
    }

    fn push_decimal(&mut self, value: u64) {
        let mut digits = [0_u8; 20];
        let mut count = 0_usize;
        let mut remaining = value;
        loop {
            let digit = remaining.checked_rem(10).unwrap_or(0);
            remaining = remaining.checked_div(10).unwrap_or(0);
            if let Some(slot) = digits.get_mut(count) {
                *slot = u8::try_from(digit).unwrap_or(0);
            }
            count = count.saturating_add(1);
            if remaining == 0 {
                break;
            }
        }
        for index in (0..count).rev() {
            if let Some(character) = digits
                .get(index)
                .copied()
                .and_then(|digit| char::from_digit(u32::from(digit), 10))
            {
                self.output.push(character);
            }
        }
    }
}

fn check_repeat_target(state: &SeqState) -> Result<(), PatternProblem> {
    match state.last {
        None | Some(PrevKind::Assertion | PrevKind::Repeat) => Err(PatternProblem::Syntax),
        Some(PrevKind::Other) => Ok(()),
    }
}

fn count_value(digits: &str) -> Result<u64, PatternProblem> {
    let mut value = 0_u64;
    for character in digits.chars() {
        let digit = u64::from(character.to_digit(10).unwrap_or(0));
        value = value.saturating_mul(10).saturating_add(digit);
    }
    if value >= MAX_REPEAT {
        return Err(PatternProblem::Syntax);
    }
    Ok(value)
}

fn is_verbose_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{0B}' | '\u{0C}')
}

fn is_flag(character: char) -> bool {
    matches!(character, 'i' | 'L' | 'm' | 's' | 'x' | 'a' | 'u')
}

fn add_add_flag(add: &mut FlagAdd, character: char) -> Result<(), PatternProblem> {
    match character {
        'i' => add.ignore_case = true,
        'm' => add.multi_line = true,
        's' => add.dot_all = true,
        'x' => add.verbose = true,
        'a' => {
            if add.unicode {
                return Err(PatternProblem::Syntax);
            }
            add.ascii = true;
        }
        'u' => {
            if add.ascii {
                return Err(PatternProblem::Syntax);
            }
            add.unicode = true;
        }
        _ => return Err(PatternProblem::Syntax),
    }
    Ok(())
}

fn add_del_flag(del: &mut FlagDel, character: char) -> Result<(), PatternProblem> {
    match character {
        'i' => del.ignore_case = true,
        'm' => del.multi_line = true,
        's' => del.dot_all = true,
        'x' => del.verbose = true,
        _ => return Err(PatternProblem::Syntax),
    }
    Ok(())
}
