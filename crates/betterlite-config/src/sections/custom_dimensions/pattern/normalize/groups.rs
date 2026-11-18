use super::super::PatternProblem;
use super::Walker;

impl Walker<'_> {
    pub(super) fn group(&mut self) -> Result<(), PatternProblem> {
        let mut probe = self.input.clone();
        let Some('?') = probe.next() else {
            return self.open_group();
        };
        match probe.next() {
            Some('#') => self.comment_group(),
            Some('<') => Err(PatternProblem::Syntax),
            Some('P') => self.python_group(),
            Some('a' | 'i' | 'L' | 'm' | 's' | 'u' | 'U' | 'R' | 'x' | '-') => self.flags_group(),
            _ => self.open_group(),
        }
    }

    fn open_group(&mut self) -> Result<(), PatternProblem> {
        self.output.push('(');
        let verbose = matches!(self.verbose.last(), Some(true));
        self.verbose.push(verbose);
        self.at_start = false;
        Ok(())
    }

    fn python_group(&mut self) -> Result<(), PatternProblem> {
        let mut probe = self.input.clone();
        probe.next();
        probe.next();
        if probe.next() != Some('<') {
            return self.open_group();
        }
        let mut name = String::new();
        loop {
            match probe.next() {
                Some('>') => break,
                Some(character) => name.push(character),
                None => return Err(PatternProblem::Syntax),
            }
        }
        if !identifier(&name) {
            return Err(PatternProblem::Syntax);
        }
        self.output.push_str("(?P<");
        self.output.push_str(&name);
        self.output.push('>');
        for _ in 0..name.chars().count() + 4 {
            let _ = self.input.next();
        }
        let verbose = matches!(self.verbose.last(), Some(true));
        self.verbose.push(verbose);
        self.at_start = false;
        Ok(())
    }

    fn flags_group(&mut self) -> Result<(), PatternProblem> {
        let mut probe = self.input.clone();
        probe.next();
        let mut spec = String::new();
        let terminator = loop {
            match probe.next() {
                Some(character @ (':' | ')')) => break character,
                Some(letter @ ('a' | 'i' | 'L' | 'm' | 's' | 'u' | 'U' | 'R' | 'x' | '-')) => {
                    spec.push(letter);
                }
                _ => return self.open_group(),
            }
        };
        let (added_x, removed_x) = flag_spec(&spec)?;
        for _ in 0..spec.chars().count() + 2 {
            let _ = self.input.next();
        }
        let inherited = matches!(self.verbose.last(), Some(true));
        let verbose = if added_x {
            true
        } else if removed_x {
            false
        } else {
            inherited
        };
        if terminator == ')' {
            // a global flag group can only turn flags on, and only at the start of the expression
            if !self.at_start || spec.contains('-') {
                return Err(PatternProblem::Syntax);
            }
            if verbose && !inherited {
                self.output.push_str("(?x)");
            }
            if let Some(base) = self.verbose.first_mut() {
                *base = verbose;
            }
            return Ok(());
        }
        if verbose {
            self.output.push_str("(?x:");
        } else if inherited {
            self.output.push_str("(?-x:");
        } else {
            // a flag group is a subpattern in the reference even when it changes no flag the cost reads
            self.output.push_str("(?u:");
        }
        self.verbose.push(verbose);
        self.at_start = false;
        Ok(())
    }

    fn comment_group(&mut self) -> Result<(), PatternProblem> {
        self.input.next();
        self.input.next();
        loop {
            match self.input.next() {
                Some('\\') => {
                    if self.input.next().is_none() {
                        return Err(PatternProblem::Syntax);
                    }
                }
                Some(')') => return Ok(()),
                Some(_) => {}
                None => return Err(PatternProblem::Syntax),
            }
        }
    }
}

fn identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first == '_' || unicode_ident::is_xid_start(first)) {
        return false;
    }
    characters.all(|character| character == '_' || unicode_ident::is_xid_continue(character))
}

fn flag_spec(spec: &str) -> Result<(bool, bool), PatternProblem> {
    let (added, removed) = spec.split_once('-').unwrap_or((spec, ""));
    if removed.contains('-') || (spec.contains('-') && removed.is_empty()) {
        return Err(PatternProblem::Syntax);
    }
    if added.contains('a') && added.contains('u') {
        return Err(PatternProblem::Syntax);
    }
    for letter in added.chars() {
        if !matches!(letter, 'a' | 'i' | 'm' | 's' | 'u' | 'x') {
            return Err(PatternProblem::Syntax);
        }
    }
    for letter in removed.chars() {
        if !matches!(letter, 'i' | 'm' | 's' | 'x') || added.contains(letter) {
            return Err(PatternProblem::Syntax);
        }
    }
    Ok((added.contains('x'), removed.contains('x')))
}
