use std::io::{self, BufRead, Write};

use regex::Regex;

#[derive(Debug, thiserror::Error)]
pub enum StyledInputError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

const ANSI_PATTERN: &str = r"\x1b\[[^m]*m";

fn render_ansi_for_terminal(prompt: &str, is_tty: bool) -> String {
    let Ok(ansi) = Regex::new(ANSI_PATTERN) else {
        return prompt.to_owned();
    };
    if is_tty {
        ansi.replace_all(prompt, "\u{1}$0\u{2}").into_owned()
    } else {
        ansi.replace_all(prompt, "").into_owned()
    }
}

pub fn styled_input(
    prompt: &str,
    is_tty: bool,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<String, StyledInputError> {
    let rendered_prompt = render_ansi_for_terminal(prompt, is_tty);
    write!(writer, "{rendered_prompt}")?;
    writer.flush()?;
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(StyledInputError::Io(io::Error::from(
            io::ErrorKind::UnexpectedEof,
        )));
    }
    Ok(line.trim().to_owned())
}

mod tests;
