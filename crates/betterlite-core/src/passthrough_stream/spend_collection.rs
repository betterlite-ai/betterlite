use std::convert::Infallible;
use std::error::Error;
use std::marker::PhantomData;
use std::pin::Pin;

use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};
use selfie::SelfieMut;
use selfie::refs::RefType;

mod tests;

pub type BoxedCollector<Logged> = Box<dyn PassthroughStreamCollector<Logged = Logged> + Send>;

pub trait SpendSink: Send + 'static {
    type Logged: Send + 'static;
    type Failure: Error + Send + Sync + 'static;

    fn add(&mut self, chunk: &[u8]) -> Result<(), Self::Failure>;

    fn into_collector(self) -> BoxedCollector<Self::Logged>;
}

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
            // Safe catch-all: spend tracking must never break the relayed stream
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

struct OwnedCollectorRef<Logged>(PhantomData<Logged>);

impl<'this, Logged: 'this> RefType<'this> for OwnedCollectorRef<Logged> {
    type Ref = Box<dyn PassthroughStreamCollector<Logged = Logged> + Send + 'this>;
}

// Owned by the flush task; the collector and the config it borrows share this cell
pub struct OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + 'static,
    C::Logged: 'static,
{
    cell: SelfieMut<'static, Box<C>, OwnedCollectorRef<C::Logged>>,
}

impl<C> OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + Sync + Unpin + 'static,
    C::Logged: 'static,
{
    pub(crate) fn new(config: C, target: PassthroughTarget<'_>) -> Self {
        let model = target.model.to_owned();
        let custom_llm_provider = target.custom_llm_provider.to_owned();
        let endpoint = target.endpoint.to_owned();
        Self {
            cell: SelfieMut::new(Box::pin(config), move |pinned| {
                let spend_target = PassthroughTarget {
                    model: &model,
                    custom_llm_provider: &custom_llm_provider,
                    endpoint: &endpoint,
                };
                Pin::into_inner(pinned).create_stream_collector(&spend_target)
            }),
        }
    }
}

impl<C> PassthroughStreamCollector for OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + 'static,
    C::Logged: 'static,
{
    type Logged = C::Logged;

    fn add(&mut self, chunk: &[u8]) {
        self.cell
            .with_referential_mut(|collector| collector.add(chunk));
    }

    fn build_logged_response(
        &self,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError> {
        self.cell
            .with_referential(|collector| collector.build_logged_response(logging_obj))
    }
}

impl<C> SpendSink for OwnedProviderSpend<C>
where
    C: BasePassthroughConfig + Send + 'static,
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
