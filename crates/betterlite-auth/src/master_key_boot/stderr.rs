use std::io::{self, Write};

pub fn announce_on_stderr(
    message: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<()> {
    stdout.flush()?;
    stderr.write_all(message.as_bytes())
}
