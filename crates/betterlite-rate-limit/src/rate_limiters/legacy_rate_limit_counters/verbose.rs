use std::io::{self, Write as _};

// litellm.set_verbose's print(statement); a failed write is reported back, never raised
pub trait VerboseConsole: Send + Sync {
    fn print_line(&self, statement: &str) -> bool;
}

// the process console LiteLLM prints to when set_verbose is on, over the writer a setup hands it
#[derive(Default)]
pub struct SystemVerboseConsole<W = io::Stdout> {
    pub writer: W,
}

impl<W> VerboseConsole for SystemVerboseConsole<W>
where
    W: Send + Sync,
    for<'writer> &'writer W: io::Write,
{
    fn print_line(&self, statement: &str) -> bool {
        writeln!(&self.writer, "{statement}").is_ok()
    }
}
