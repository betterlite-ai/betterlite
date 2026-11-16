#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    MAX_COLLECTED_BYTES, PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};
use tracing::Level;
use url::Url;

use super::{OwnedProviderSpend, SpendCollection};
use crate::passthrough_stream::tests::{
    ENDPOINT, FakeConfigError, MODEL, NoopPassthroughLogging, PROVIDER, RecordingCollector,
    capture, collection, fake_config,
};

fn owned_spend<C>(config: C) -> OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + Sync + Unpin + 'static,
    C::Logged: 'static,
{
    OwnedProviderSpend::new(
        config,
        PassthroughTarget {
            model: MODEL,
            custom_llm_provider: PROVIDER,
            endpoint: ENDPOINT,
        },
    )
}

type TargetStrings = (String, String, String);

struct CountingConfig {
    created: Arc<Mutex<Vec<TargetStrings>>>,
}

impl BasePassthroughConfig for CountingConfig {
    type Error = FakeConfigError;
    type Logged = String;

    fn is_streaming_request(&self, _endpoint: &str, _request_data: &Object) -> bool {
        true
    }

    fn get_complete_url(
        &self,
        _request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let base = "https://api.test";
        Ok((Url::parse(base).unwrap(), base.to_owned()))
    }

    fn create_stream_collector(
        &self,
        target: &PassthroughTarget<'_>,
    ) -> Box<dyn PassthroughStreamCollector<Logged = Self::Logged> + Send + '_>
    where
        Self: Sized + Sync,
    {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                target.model.to_owned(),
                target.custom_llm_provider.to_owned(),
                target.endpoint.to_owned(),
            ));
        Box::new(RecordingCollector { chunks: Vec::new() })
    }
}

#[test]
fn owned_spend_hands_chunks_to_the_provider_collector() {
    let config = fake_config();
    let collected = Arc::clone(&config.collected);
    let mut spend = SpendCollection {
        sink: Some(owned_spend(config)),
        chunk_count: 0,
        failed: false,
    };
    spend.add(b"data: one\n\n");
    spend.add(b"data: two\n\n");
    let (collector, chunk_count) = spend.take_flush().expect("two chunks flush");
    assert_eq!(chunk_count, 2);
    let mut logging = NoopPassthroughLogging;
    let logged = collector
        .build_logged_response(&mut logging)
        .expect("the provider collector builds")
        .expect("the provider collector logs");
    assert_eq!(logged, "data: one;data: two");
    let records = collected
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model, MODEL);
    assert_eq!(records[0].custom_llm_provider, PROVIDER);
    assert_eq!(records[0].endpoint, ENDPOINT);
    assert_eq!(
        records[0].lines,
        vec!["data: one".to_owned(), "data: two".to_owned()]
    );
}

#[test]
fn owned_spend_builds_the_provider_collector_once_at_construction() {
    let created = Arc::new(Mutex::new(Vec::new()));
    let mut sink = owned_spend(CountingConfig {
        created: Arc::clone(&created),
    });
    {
        let targets = created.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            *targets,
            vec![(MODEL.to_owned(), PROVIDER.to_owned(), ENDPOINT.to_owned())]
        );
    }
    PassthroughStreamCollector::add(&mut sink, b"data: one\n\n");
    let mut logging = NoopPassthroughLogging;
    let logged = sink
        .build_logged_response(&mut logging)
        .expect("the provider collector builds");
    assert_eq!(logged.as_deref(), Some("data: one\n\n"));
    assert_eq!(
        created.lock().unwrap_or_else(PoisonError::into_inner).len(),
        1
    );
}

#[test]
fn add_absorbs_a_collector_failure_and_drops_later_chunks() {
    let (mut spend, state) = collection();
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    let (captured, _guard) = capture();
    spend.add(b"one");
    spend.add(b"two");
    {
        let guarded = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(guarded.calls, 1);
        assert!(guarded.chunks.is_empty());
    }
    assert!(spend.take_flush().is_none());
    let errors: Vec<_> = captured
        .events()
        .into_iter()
        .filter(|event| event.level == Level::ERROR)
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .message
            .contains("Passthrough spend-tracking collector failed; spend dropped for this stream")
    );
    assert!(errors[0].fields.iter().any(|field| field == "chunks=1"));
    assert!(
        errors[0]
            .fields
            .iter()
            .any(|field| field.contains("collector add 1 failed"))
    );
}

#[test]
fn should_flush_needs_chunks_and_a_successful_collector() {
    let (mut empty, _state) = collection();
    assert!(empty.take_flush().is_none());
    let (mut kept, _state) = collection();
    kept.add(b"a");
    assert!(kept.take_flush().is_some());
    let (mut failed, state) = collection();
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    failed.add(b"a");
    assert!(failed.take_flush().is_none());
}

#[test]
fn take_flush_hands_the_collector_over_once() {
    let (mut spend, _state) = collection();
    spend.add(b"a");
    assert!(spend.take_flush().is_some());
    assert!(spend.take_flush().is_none());
    spend.add(b"b");
    assert!(spend.take_flush().is_none());
}

#[test]
fn owned_spend_stops_collecting_exactly_past_the_byte_cap() {
    let config = CountingConfig {
        created: Arc::new(Mutex::new(Vec::new())),
    };
    let mut sink = owned_spend(config);
    PassthroughStreamCollector::add(&mut sink, &vec![b'a'; MAX_COLLECTED_BYTES]);
    let mut logging = NoopPassthroughLogging;
    assert!(
        sink.build_logged_response(&mut logging)
            .expect("the collector builds at the cap")
            .is_some(),
        "a chunk that ends exactly on the cap is still collected"
    );
    PassthroughStreamCollector::add(&mut sink, b"b");
    assert!(
        sink.build_logged_response(&mut logging)
            .expect("a disabled collector builds nothing")
            .is_none(),
        "a chunk past the cap stops collection"
    );
}
