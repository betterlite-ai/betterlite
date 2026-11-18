use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use encoding_rs::{Decoder, UTF_8};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use super::setup::{BeatStream, Ports, Setup, list_beats, request_object};
use super::types::{
    CleanupOutcome, Flag, SseDataGenerator, StreamCleanup, StreamErrorState, StreamFailure,
    StreamHooks, StreamSource,
};
use crate::data_generator_select::GeneratorInput;
use crate::stream_keepalive::wrapper::Beat;

pub(super) struct Session<Key>
where
    Key: Sync,
{
    pub(super) hooks: Arc<dyn StreamHooks<Key>>,
    pub(super) cleanup: Arc<dyn StreamCleanup<Key>>,
    pub(super) request_data: Arc<JsonObject>,
    pub(super) error_state: Option<Box<dyn StreamErrorState + Send>>,
    pub(super) outcome: Option<CleanupOutcome<Key>>,
    pub(super) beats: BeatStream,
    pub(super) setup: Setup,
}

pub(super) struct Drive<Key>
where
    Key: Sync,
{
    pub(super) session: Session<Key>,
    pub(super) fallback_sent: Flag,
    pub(super) mismatch: Flag,
    pub(super) finished: Flag,
    pub(super) chunk_text: String,
    pub(super) raw_buffer: String,
    pub(super) decoder: Decoder,
    pub(super) error_message: Option<String>,
    pub(super) pending_failure: Option<StreamFailure>,
}

pub(super) enum Output {
    Item(String),
    Raw(Vec<String>),
    Overflow(Vec<String>),
    End,
    Held,
}

pub(super) fn generate<Key>(
    generator: &SseDataGenerator<Key>,
    input: GeneratorInput<StreamSource, Key>,
) -> BoxStream<'static, Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    let ports = Ports {
        hooks: Arc::clone(&generator.hooks),
        error_state: Arc::clone(&generator.error_state),
        cleanup: Arc::clone(&generator.cleanup),
        router: generator.router.as_ref().map(Arc::clone),
        global_keepalive: generator.global_keepalive.clone(),
        expose_fallback_errors: Flag(generator.expose_fallback_errors),
        is_model_router: generator.is_model_router,
    };
    async_stream::stream! {
        let mut drive = Drive::start(ports, input);
        while let Some(frames) = drive.next().await {
            for frame in frames {
                yield frame;
            }
        }
    }
    .boxed()
}

impl<Key> Drive<Key>
where
    Key: Send + Sync + 'static,
{
    pub(super) fn start(ports: Ports<Key>, input: GeneratorInput<StreamSource, Key>) -> Self {
        let GeneratorInput {
            response,
            key,
            request_data,
            request,
            responses_stream_errors,
        } = input;
        let mut source = response;
        let snapshot = source.snapshot.take();
        let mut session = Session::open(ports, source, request_data, responses_stream_errors, &key);
        session.outcome = Some(CleanupOutcome {
            request,
            request_data: JsonObject::clone(session.request_data.as_ref()),
            key,
            response: snapshot,
            stream_completed: false,
            client_disconnected: false,
        });
        Self {
            session,
            fallback_sent: Flag(false),
            mismatch: Flag(false),
            finished: Flag(false),
            chunk_text: String::new(),
            raw_buffer: String::new(),
            decoder: UTF_8.new_decoder_without_bom_handling(),
            error_message: None,
            pending_failure: None,
        }
    }

    pub(super) async fn next(&mut self) -> Option<Vec<Result<Bytes, io::Error>>> {
        if self.finished.0 {
            super::cleanup::finish(&mut self.session, true);
            return None;
        }
        if let Some(failure) = self.pending_failure.take() {
            return Some(super::finish::handle_failure(self, failure).await);
        }
        let beat = self.session.beats.next().await;
        let frames = match beat {
            None => super::item::end_source(self),
            Some(Beat::Ping) => vec![Ok(Bytes::from_static(b": ping\n\n"))],
            Some(Beat::Item(Err(failure))) => super::finish::handle_failure(self, failure).await,
            Some(Beat::Item(Ok(item))) => super::item::handle_item(self, item).await,
        };
        Some(frames)
    }
}

impl<Key> Session<Key>
where
    Key: Sync,
{
    pub(super) fn open(
        ports: Ports<Key>,
        source: StreamSource,
        request_data: BTreeMap<String, JsonValue>,
        responses_stream_errors: bool,
        key: &Key,
    ) -> Self {
        let error_state = responses_stream_errors.then(|| ports.error_state.new_state());
        let keepalive_data = Arc::new(request_data);
        let converted = Arc::new(request_object(&keepalive_data));
        let setup = Setup::read(
            ports.hooks.as_ref(),
            ports.expose_fallback_errors,
            &converted,
            &source.hidden,
            ports.is_model_router,
        );
        let items = ports.wrapped_items(source.items, key, &converted, setup.wrapped);
        let beats = list_beats(
            ports.router,
            ports.global_keepalive,
            keepalive_data,
            &source.hidden,
            items,
        );
        Self {
            hooks: ports.hooks,
            cleanup: ports.cleanup,
            request_data: converted,
            error_state,
            outcome: None,
            beats,
            setup,
        }
    }
}
