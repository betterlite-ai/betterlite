use std::convert::Infallible;
use std::error::Error;

use betterlite_openai::passthrough::transformation::types::{
    MAX_COLLECTED_BYTES, PassthroughError, PassthroughLogging, PassthroughTarget,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};

mod tests;

pub type BoxedCollector<Logged> = Box<dyn PassthroughStreamCollector<Logged = Logged> + Send>;

// A collector whose add can raise the way LiteLLM's collector.add can
pub trait SpendSink: Send + 'static {
    type Logged: Send + 'static;
    type Failure: Error + Send + Sync + 'static;

    fn add(&mut self, chunk: &[u8]) -> Result<(), Self::Failure>;

    fn into_collector(self) -> BoxedCollector<Self::Logged>;
}

// LiteLLM's _SpendCollection: feeds the collector until one add raises, then drops the rest of the stream's spend
pub struct SpendCollection<S: SpendSink> {
    pub(crate) sink: Option<S>,
    pub(crate) chunk_count: usize,
    pub(crate) failed: bool,
}

impl<S: SpendSink> SpendCollection<S> {
    pub fn add(&mut self, chunk: &[u8]) {
        self.chunk_count = self.chunk_count.saturating_add(1);
        if self.failed {
            return;
        }
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        if let Err(error) = sink.add(chunk) {
            self.failed = true;
            tracing::error!(
                chunks = self.chunk_count,
                %error,
                "Passthrough spend-tracking collector failed; spend dropped for this stream"
            );
        }
    }

    pub fn take_flush(&mut self) -> Option<(BoxedCollector<S::Logged>, usize)> {
        if self.failed || self.chunk_count == 0 {
            return None;
        }
        let sink = self.sink.take()?;
        Some((sink.into_collector(), self.chunk_count))
    }
}

// The provider's collector, owned so the background flush outlives the request; it copies the bytes it feeds back in
pub struct OwnedProviderSpend<C> {
    pub(crate) config: C,
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) endpoint: String,
    pub(crate) chunks: Vec<Vec<u8>>,
    pub(crate) collected_bytes: usize,
    pub(crate) disabled: bool,
}

impl<C> PassthroughStreamCollector for OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + Sync,
{
    type Logged = C::Logged;

    fn add(&mut self, chunk: &[u8]) {
        if self.disabled {
            return;
        }
        let Some(total) = self
            .collected_bytes
            .checked_add(chunk.len())
            .filter(|total| *total <= MAX_COLLECTED_BYTES)
        else {
            self.disabled = true;
            self.chunks.clear();
            self.collected_bytes = 0;
            return;
        };
        self.chunks.push(chunk.to_vec());
        self.collected_bytes = total;
    }

    fn build_logged_response(
        &self,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError> {
        if self.disabled {
            return Ok(None);
        }
        let mut collector = self.config.create_stream_collector(&PassthroughTarget {
            model: &self.model,
            custom_llm_provider: &self.custom_llm_provider,
            endpoint: &self.endpoint,
        });
        for chunk in &self.chunks {
            collector.add(chunk);
        }
        collector.build_logged_response(logging_obj)
    }
}

impl<C> SpendSink for OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + Sync + Send + 'static,
    C::Logged: Send + 'static,
{
    type Logged = C::Logged;
    type Failure = Infallible;

    fn add(&mut self, chunk: &[u8]) -> Result<(), Infallible> {
        PassthroughStreamCollector::add(self, chunk);
        Ok(())
    }

    fn into_collector(self) -> BoxedCollector<C::Logged> {
        Box::new(self)
    }
}

// LiteLLM's _SpendCollection.__init__: the collector is built over the logging object's model, provider and endpoint
#[must_use]
pub fn provider_spend<C>(
    config: C,
    target: PassthroughTarget<'_>,
) -> SpendCollection<OwnedProviderSpend<C>>
where
    C: BasePassthroughConfig + Sync + Send + 'static,
    C::Logged: Send + 'static,
{
    SpendCollection {
        sink: Some(OwnedProviderSpend {
            config,
            model: target.model.to_owned(),
            custom_llm_provider: target.custom_llm_provider.to_owned(),
            endpoint: target.endpoint.to_owned(),
            chunks: Vec::new(),
            collected_bytes: 0,
            disabled: false,
        }),
        chunk_count: 0,
        failed: false,
    }
}
