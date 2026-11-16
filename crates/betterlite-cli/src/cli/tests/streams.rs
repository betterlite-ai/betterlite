use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};

use crate::cli::tests::support::channel;

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn events(&self) -> Vec<String> {
        let events = self.0.lock().unwrap();
        events.clone()
    }

    fn record(&self, event: String) {
        let mut events = self.0.lock().unwrap();
        events.push(event);
    }
}

impl io::Write for Recorder {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.record(format!(
            "write:{}",
            String::from_utf8(buffer.to_vec()).unwrap_or_default()
        ));
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.record(String::from("flush"));
        Ok(())
    }
}

#[test]
fn a_channel_write_reports_the_bytes_it_took() {
    let recorder = Recorder::default();
    let observed = recorder.clone();
    let mut sink = channel(Box::new(recorder));
    assert_eq!(sink.write(b"three").unwrap(), 5);
    assert_eq!(observed.events(), ["write:three"]);
}

#[test]
fn a_channel_flush_reaches_the_sink() {
    let recorder = Recorder::default();
    let observed = recorder.clone();
    let mut sink = channel(Box::new(recorder));
    sink.flush().unwrap();
    assert_eq!(observed.events(), ["flush"]);
}

#[test]
fn channels_cloned_from_one_stream_share_the_sink() {
    let recorder = Recorder::default();
    let observed = recorder.clone();
    let mut sink = channel(Box::new(recorder));
    let mut clone = sink.clone();
    clone.write_all(b"shared").unwrap();
    sink.write_all(b"-tail").unwrap();
    assert_eq!(observed.events(), ["write:shared", "write:-tail"]);
}
