use std::io;

pub trait OperatorOutput {
    fn instructions(&mut self, message: &str);
}

#[derive(Debug, Default)]
pub struct StreamOutput<W> {
    pub writer: W,
}

impl<W: io::Write> OperatorOutput for StreamOutput<W> {
    fn instructions(&mut self, message: &str) {
        if writeln!(self.writer, "{message}")
            .and_then(|()| self.writer.flush())
            .is_err()
        {
            tracing::warn!("the Copilot device login instructions could not be written");
        }
    }
}

pub type StdoutOutput = StreamOutput<io::Stdout>;

mod tests;
