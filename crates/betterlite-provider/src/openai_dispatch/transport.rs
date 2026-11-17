use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::text_completion_execution::http::HttpCompletionConstructor;
use betterlite_openai::text_completion_transport::sdk::ClientConstructor;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::operation::OperationError;
use crate::prepared::{PreparedRequest, Upstream};

// The provider answer the chat and embedding branches read; the production client sends the prepared request, fixtures answer it.
pub trait ProviderAnswer: Sync {
    fn answer(
        &self,
        prepared: &PreparedRequest,
    ) -> impl Future<Output = Result<Upstream, OperationError>> + Send;
}

// The pooled outbound client, which is the only transport a deployment's prepared request needs.
pub struct ClientAnswer {
    pub client: ProxiedClient,
}

impl ProviderAnswer for ClientAnswer {
    async fn answer(&self, prepared: &PreparedRequest) -> Result<Upstream, OperationError> {
        prepared.send(&self.client).await
    }
}

// The text branch builds its SDK constructor from the key it resolved for this call, never from a module-level value.
pub trait TextClientSource: Sync {
    // the constructor travels through the transport's send, so it must cross that await
    type Constructor: ClientConstructor + Send;

    fn constructor(&self, api_key: Option<&SecretString>) -> Self::Constructor;
}

// The production source: B2238's HTTP constructor over the same pooled client, authorized with this call's key.
pub struct HttpTextClients {
    pub client: ProxiedClient,
}

impl TextClientSource for HttpTextClients {
    type Constructor = HttpCompletionConstructor;

    fn constructor(&self, api_key: Option<&SecretString>) -> HttpCompletionConstructor {
        HttpCompletionConstructor {
            client: self.client.clone(),
            authorization: api_key
                .map(|key| SecretString::from(format!("Bearer {}", key.expose_secret()))),
            facts: Arc::new(Mutex::new(None)),
        }
    }
}

// The injected ports and clocks one dispatch call reads; the clock keeps a projection's created stamp stable in tests.
pub struct Deps<'source, Answer: Sync, Text> {
    pub answer: &'source Answer,
    pub text: &'source Text,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}
