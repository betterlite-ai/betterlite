use std::io::{self, BufRead, Write};

use regex::Regex;

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
) -> io::Result<String> {
    let rendered_prompt = render_ansi_for_terminal(prompt, is_tty);
    write!(writer, "{rendered_prompt}")?;
    writer.flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

mod tests;
