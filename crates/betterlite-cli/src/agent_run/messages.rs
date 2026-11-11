use crate::agent_launch::warn;
use std::io::{self, Write};
use std::str;
pub trait Messages {
    fn warn(&mut self, message: &str) -> io::Result<()>;
    fn notice(&mut self, message: &str) -> io::Result<()>;
}
pub struct StreamMessages;
impl Messages for StreamMessages {
    fn warn(&mut self, message: &str) -> io::Result<()> {
        warn(&mut io::stderr(), message)
    }
    fn notice(&mut self, message: &str) -> io::Result<()> {
        warn(&mut io::stdout(), message)
    }
}
pub struct NoticeBuffer<'messages> {
    pub messages: &'messages mut dyn Messages,
    pub text: String,
}
impl NoticeBuffer<'_> {
    pub fn finish(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let message = self.text.trim_end_matches('\n');
        let _unused = self.messages.notice(message);
        self.text.clear();
    }
}
impl Write for NoticeBuffer<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.text.push_str(text);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
