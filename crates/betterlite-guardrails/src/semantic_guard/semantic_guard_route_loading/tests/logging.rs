use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::semantic_guard::semantic_guard_route_loading::build_routes;

use super::support::{array, object, sources, text};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn build_routes_logs_the_route_count() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    with_default(subscriber, || build_routes(&sources)).unwrap();
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("SemanticGuard: built 1 routes"));
}
