#![cfg(test)]

use std::cell::Cell;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::otel_provider_assembly::error::ProviderLookupError;
use crate::otel_provider_assembly::opentelemetry_legacy_provider::{
    GlobalProviders, GlobalSlot, ProviderSlot, get_or_create_provider,
};

const CREATED: u32 = 9;
const EXISTING: u32 = 7;
const PROVIDED: u32 = 3;

#[derive(Clone)]
enum Lookup {
    Nothing,
    Found,
    Broken,
}

struct ScriptedSlot {
    lookup: Lookup,
    lookups: Cell<usize>,
    published: Mutex<Vec<u32>>,
}

impl ScriptedSlot {
    fn new(lookup: Lookup) -> Self {
        Self {
            lookup,
            lookups: Cell::new(0),
            published: Mutex::new(Vec::new()),
        }
    }
}

impl ProviderSlot for ScriptedSlot {
    type Provider = u32;

    fn name(&self) -> &'static str {
        "ScriptedProvider"
    }

    fn existing(&self) -> Result<Option<u32>, ProviderLookupError> {
        self.lookups.set(self.lookups.get().saturating_add(1));
        match self.lookup {
            Lookup::Nothing => Ok(None),
            Lookup::Found => Ok(Some(EXISTING)),
            Lookup::Broken => Err(ProviderLookupError {
                provider: "ScriptedProvider",
            }),
        }
    }

    fn publish(&self, provider: &u32) {
        self.published.lock().unwrap().push(*provider);
    }
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

struct Outcome {
    provider: u32,
    created: usize,
    lookups: usize,
    published: Vec<u32>,
    log: String,
}

fn run(lookup: Lookup, provided: Option<u32>, skip_set_global: bool) -> Outcome {
    let slot = ScriptedSlot::new(lookup);
    let created = Cell::new(0_usize);
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let resolved = with_default(subscriber, || {
        get_or_create_provider(&slot, provided, skip_set_global, || {
            created.set(created.get().saturating_add(1));
            Ok::<u32, String>(CREATED)
        })
        .expect("the scripted create succeeds")
    });
    let log = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    Outcome {
        provider: resolved,
        created: created.get(),
        lookups: slot.lookups.get(),
        published: slot.published.lock().unwrap().clone(),
        log,
    }
}

#[rstest]
#[case::set_global(false)]
#[case::skip_set_global(true)]
fn a_provided_provider_is_returned_without_a_lookup_or_publication(#[case] skip: bool) {
    let outcome = run(Lookup::Found, Some(PROVIDED), skip);
    assert_eq!(outcome.provider, PROVIDED);
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.lookups, 0);
    assert!(outcome.published.is_empty());
    assert!(outcome.log.contains("using the provided provider"));
}

#[test]
fn an_existing_provider_is_reused_when_global_publication_is_allowed() {
    let outcome = run(Lookup::Found, None, false);
    assert_eq!(outcome.provider, EXISTING);
    assert_eq!(outcome.created, 0);
    assert!(outcome.published.is_empty());
    assert!(outcome.log.contains("using the existing provider"));
}

#[test]
fn skip_set_global_builds_a_private_provider_beside_an_existing_one() {
    let outcome = run(Lookup::Found, None, true);
    assert_eq!(outcome.provider, CREATED);
    assert_eq!(outcome.created, 1);
    assert!(outcome.published.is_empty());
    assert!(outcome.log.contains("creating a private one for isolation"));
}

#[test]
fn a_missing_provider_is_created_and_published() {
    let outcome = run(Lookup::Nothing, None, false);
    assert_eq!(outcome.provider, CREATED);
    assert_eq!(outcome.created, 1);
    assert_eq!(outcome.published, vec![CREATED]);
    assert!(outcome.log.contains("creating a new provider"));
    assert!(!outcome.log.contains("NOT setting it globally"));
}

#[test]
fn a_missing_provider_is_not_published_when_skip_set_global_is_set() {
    let outcome = run(Lookup::Nothing, None, true);
    assert_eq!(outcome.provider, CREATED);
    assert_eq!(outcome.created, 1);
    assert!(outcome.published.is_empty());
    assert!(outcome.log.contains("NOT setting it globally"));
}

#[test]
fn a_failed_lookup_creates_a_provider_publishes_it_and_logs_the_error() {
    let outcome = run(Lookup::Broken, None, false);
    assert_eq!(outcome.provider, CREATED);
    assert_eq!(outcome.created, 1);
    assert_eq!(outcome.published, vec![CREATED]);
    assert!(
        outcome
            .log
            .contains("exception checking the existing provider")
    );
    assert!(
        outcome
            .log
            .contains("the global ScriptedProvider registry is poisoned")
    );
}

#[test]
fn a_failed_lookup_does_not_publish_when_skip_set_global_is_set() {
    let outcome = run(Lookup::Broken, None, true);
    assert_eq!(outcome.provider, CREATED);
    assert_eq!(outcome.created, 1);
    assert!(outcome.published.is_empty());
}

#[rstest]
#[case::nothing(Lookup::Nothing)]
#[case::broken(Lookup::Broken)]
fn a_failed_create_returns_its_error_and_publishes_nothing(#[case] lookup: Lookup) {
    let slot = ScriptedSlot::new(lookup);
    let result = get_or_create_provider(&slot, None, false, || Err::<u32, &str>("exporter failed"));
    assert_eq!(result, Err("exporter failed"));
    assert!(slot.published.lock().unwrap().is_empty());
}

#[test]
fn a_published_provider_is_found_by_the_next_lookup() {
    let slot = GlobalSlot {
        name: "TracerProvider",
        current: Mutex::new(None),
    };
    assert_eq!(slot.existing().unwrap(), None);
    slot.publish(&CREATED);
    assert_eq!(slot.existing().unwrap(), Some(CREATED));
    assert_eq!(slot.name(), "TracerProvider");
}

#[test]
fn a_poisoned_slot_fails_the_lookup_and_still_takes_a_publication() {
    let slot = GlobalSlot {
        name: "MeterProvider",
        current: Mutex::new(None),
    };
    let joined = thread::scope(|scope| {
        scope
            .spawn(|| {
                let _held = slot.current.lock().unwrap();
                panic!("poison the slot");
            })
            .join()
    });
    assert!(joined.is_err());
    let error = slot.existing().unwrap_err();
    assert_eq!(error.provider, "MeterProvider");
    slot.publish(&PROVIDED);
    let recovered = slot.current.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(*recovered, Some(PROVIDED));
}

#[test]
fn the_default_registry_names_each_provider_kind() {
    let providers = GlobalProviders::default();
    assert_eq!(providers.tracer.name, "TracerProvider");
    assert_eq!(providers.meter.name, "MeterProvider");
    assert_eq!(providers.logger.name, "LoggerProvider");
    assert!(providers.tracer.existing().unwrap().is_none());
    assert!(providers.meter.existing().unwrap().is_none());
    assert!(providers.logger.existing().unwrap().is_none());
}
