#![cfg(test)]
use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::parse_headers;

#[rstest]
#[case::empty(None, &[])]
#[case::blank(Some(""), &[])]
#[case::plain(Some("a=b,c=d"), &[("a", "b"), ("c", "d")])]
#[case::name_lowercased(Some("X-Key=Value"), &[("x-key", "Value")])]
#[case::percent_decoded(Some("authorization=Basic%20dG9rZW4%3D"), &[("authorization", "Basic dG9rZW4=")])]
#[case::encoded_value_edges_trimmed(Some("a=%20v%20"), &[("a", "v")])]
#[case::encoded_name_edges_trimmed(Some("%20a%20=v"), &[("a", "v")])]
#[case::plus_kept(Some("a=1+2"), &[("a", "1+2")])]
#[case::liberal_space(Some("authorization=Bearer abc def"), &[("authorization", "Bearer abc def")])]
#[case::liberal_keeps_percent(Some("a=x y%20z"), &[("a", "x y%20z")])]
#[case::equals_in_value(Some("a=b=c"), &[("a", "b=c")])]
#[case::spaces_around(Some(" a = b , c=d "), &[("a", "b"), ("c", "d")])]
#[case::last_wins(Some("a=1,a=2"), &[("a", "2")])]
#[case::no_equals_dropped(Some("novalue,a=b"), &[("a", "b")])]
#[case::empty_name_dropped(Some("=v,a=b"), &[("a", "b")])]
#[case::bad_name_dropped(Some("a b=v,c=d"), &[("c", "d")])]
#[case::empty_value_kept(Some("a="), &[("a", "")])]
#[case::semicolon_dropped(Some("a=b;c,d=e"), &[("d", "e")])]
fn exporter_headers_parse_like_the_sdk(
    #[case] raw: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let owned = expected
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(parse_headers(raw), owned);
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.push(fields.0.join(" "));
        }
    }
}

#[derive(Default)]
struct Fields(Vec<String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.push(format!("{}={value}", field.name()));
    }
}

#[test]
fn exporter_headers_warn_when_an_entry_is_dropped() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&events)));
    let headers = with_default(subscriber, || {
        parse_headers(Some("a=b,Authorization: Bearer sk-live-secret"))
    });
    assert_eq!(headers.len(), 1);
    assert!(headers.contains_key("a"));
    let captured = events.lock().expect("events");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("position=2"));
    assert!(!captured[0].contains("sk-live-secret"));
    drop(captured);
}

#[rstest]
fn replays_the_exporter_headers_corpus(#[files("fuzz/corpus/parse_headers/*")] path: PathBuf) {
    let data = fs::read(&path).expect("corpus file");
    let Ok(text) = str::from_utf8(&data) else {
        return;
    };
    drop(parse_headers(Some(text)));
}
