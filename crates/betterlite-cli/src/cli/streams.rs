use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone)]
pub(crate) struct Channel(pub(crate) Arc<Mutex<Box<dyn Write + Send>>>);

impl Write for Channel {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut sink = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sink.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut sink = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sink.flush()
    }
}

#[derive(Clone)]
pub(crate) struct Streams {
    pub(crate) stdout: Channel,
    pub(crate) stderr: Channel,
}

impl Streams {
    #[must_use]
    pub(crate) fn new(stdout: Box<dyn Write + Send>, stderr: Box<dyn Write + Send>) -> Self {
        Self {
            stdout: Channel(Arc::new(Mutex::new(stdout))),
            stderr: Channel(Arc::new(Mutex::new(stderr))),
        }
    }
}
