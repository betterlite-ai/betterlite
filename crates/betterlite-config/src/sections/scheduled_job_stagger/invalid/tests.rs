#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;

use crate::loader::ConfigLoader;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
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

fn load_with_capture(source: &str) -> (bool, String) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .finish();
    let loaded = with_default(subscriber, || {
        ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load(source)
    });
    let output = String::from_utf8(
        captured
            .0
            .lock()
            .expect("capture mutex should not be poisoned")
            .clone(),
    )
    .expect("captured log output should be UTF-8");
    (loaded.is_ok(), output)
}

#[test]
fn negative_window_falls_back() {
    let (loaded, output) =
        load_with_capture("general_settings:\n  scheduled_job_stagger:\n    window_seconds: -1\n");
    assert!(loaded, "startup must not fail on an invalid stagger config");
    assert!(output.contains("falling back to defaults"));
}

#[test]
fn wrong_shape_falls_back() {
    let (loaded, output) =
        load_with_capture("general_settings:\n  scheduled_job_stagger: not-a-mapping\n");
    assert!(loaded, "startup must not fail on an invalid stagger config");
    assert!(output.contains("falling back to defaults"));
}

#[test]
fn unrecognized_boolean_word_falls_back() {
    let (loaded, output) =
        load_with_capture("general_settings:\n  scheduled_job_stagger:\n    enabled: nah\n");
    assert!(loaded, "startup must not fail on an invalid stagger config");
    assert!(output.contains("falling back to defaults"));
}

#[test]
fn negative_window_string_falls_back() {
    let (loaded, output) = load_with_capture(
        "general_settings:\n  scheduled_job_stagger:\n    window_seconds: \"-1\"\n",
    );
    assert!(loaded, "startup must not fail on an invalid stagger config");
    assert!(output.contains("falling back to defaults"));
}

#[test]
fn leading_underscore_integer_string_falls_back() {
    let (loaded, output) = load_with_capture(
        "general_settings:\n  scheduled_job_stagger:\n    offsets:\n      custom_cron: \"_1\"\n",
    );
    assert!(loaded, "startup must not fail on an invalid stagger config");
    assert!(output.contains("falling back to defaults"));
}

#[test]
fn invalid_config_uses_defaults() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  scheduled_job_stagger:\n    window_seconds: -1\n")
    .expect("config should fall back instead of failing to load");
    let settings = &config.sections.scheduled_job_stagger;
    assert!(settings.enabled);
    assert_eq!(settings.window_seconds, 300);
}
