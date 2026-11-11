use std::io::{self, BufRead as _, IsTerminal as _};

use console::{Key, Term};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyPress {
    Up,
    Down,
    Enter,
    Escape,
    Quit,
}

pub trait SelectionInput {
    fn is_interactive(&self) -> bool;
    fn read_key(&mut self) -> Option<Key>;
    fn read_line(&mut self) -> Option<String>;
}

pub struct Terminal {
    term: Term,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            term: Term::stdout(),
        }
    }
}

impl SelectionInput for Terminal {
    #[cfg_attr(test, mutants::skip)]
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal() && self.term.is_term()
    }

    #[cfg_attr(test, mutants::skip)]
    fn read_key(&mut self) -> Option<Key> {
        self.term.read_key().ok()
    }

    #[cfg_attr(test, mutants::skip)]
    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        let read = io::stdin().lock().read_line(&mut line).ok()?;
        if read == 0 {
            return None;
        }
        Some(line.trim().to_owned())
    }
}

#[must_use]
pub fn get_key_input(input: &mut dyn SelectionInput) -> Option<KeyPress> {
    let key = input.read_key()?;
    if key == Key::ArrowUp {
        Some(KeyPress::Up)
    } else if key == Key::ArrowDown {
        Some(KeyPress::Down)
    } else if key == Key::Enter {
        Some(KeyPress::Enter)
    } else if key == Key::Escape {
        Some(KeyPress::Escape)
    } else if key == Key::Char('q') {
        Some(KeyPress::Quit)
    } else {
        None
    }
}
