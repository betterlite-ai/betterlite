use std::io::{self, Write as _};

// litellm.set_verbose's print(statement); a failed write is reported back, never raised
pub trait VerboseConsole: Send + Sync {
    fn print_line(&self, statement: &str) -> bool;
}

// the process console LiteLLM prints to when set_verbose is on
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemVerboseConsole;

impl VerboseConsole for SystemVerboseConsole {
    fn print_line(&self, statement: &str) -> bool {
        let mut stdout = io::stdout();
        writeln!(stdout, "{statement}").is_ok()
    }
}
