use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone)]
pub(crate) struct Channel(Arc<Mutex<Box<dyn Write + Send>>>);

impl Channel {
    #[must_use]
    pub(crate) fn new(sink: Box<dyn Write + Send>) -> Self {
        Self(Arc::new(Mutex::new(sink)))
    }
}

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
    pub stdout: Channel,
    pub stderr: Channel,
}

impl Streams {
    #[must_use]
    pub(crate) fn process() -> Self {
        Self {
            stdout: Channel::new(Box::new(io::stdout())),
            stderr: Channel::new(Box::new(io::stderr())),
        }
    }
}
