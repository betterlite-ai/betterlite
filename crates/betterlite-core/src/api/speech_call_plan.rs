use std::future::Future;

use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};

use error::AspeechError;

pub mod error;
mod tests;

// exception_type's provider when neither the wrapper's resolver nor the caller named one
const FALLBACK_PROVIDER: &str = "openai";

// what speech() hands aspeech: a coroutine to await, or a value it already finished
#[derive(Debug)]
pub enum SpeechReturn<Answer, Pending> {
    Coroutine(Pending),
    Finished(Answer),
}

// the keywords aspeech reads itself; everything else reaches speech() as the caller passed it
pub trait AspeechKeywords {
    // args[0], else kwargs["model"]; None is a call that passed neither
    fn model(&self) -> Option<&str>;

    fn api_base(&self) -> Option<&str>;

    fn custom_llm_provider(&self) -> Option<&str>;

    // kwargs["aspeech"] = True, whatever the caller passed
    fn force_async(&mut self);
}

// speech() as aspeech's executor runs it; both invocations share the one request context
pub trait SpeechInvoker<Keywords>: Sync {
    type Context: Sync;
    type Answer: Send;
    type Error: From<ResolutionError> + Send;
    type Pending: Future<Output = Result<Self::Answer, Self::Error>> + Send;

    fn speech(
        &self,
        keywords: &Keywords,
        context: &Self::Context,
    ) -> impl Future<Output = Result<SpeechReturn<Self::Answer, Self::Pending>, Self::Error>> + Send;
}

pub struct AsyncSpeech<'service, Invoker> {
    pub invoker: &'service Invoker,
    pub resolution: &'service ResolutionContext<'service>,
}

impl<Invoker> AsyncSpeech<'_, Invoker> {
    // aspeech(): a coroutine is awaited, and any other first result is dropped for a second speech() call
    pub async fn aspeech<Keywords>(
        &self,
        mut keywords: Keywords,
        context: &Invoker::Context,
    ) -> Result<SpeechReturn<Invoker::Answer, Invoker::Pending>, AspeechError<Invoker::Error>>
    where
        Keywords: AspeechKeywords + Send + Sync,
        Invoker: SpeechInvoker<Keywords>,
    {
        let model = keywords
            .model()
            .ok_or(AspeechError::MissingModel)?
            .to_owned();
        keywords.force_async();
        // the wrapper's get_llm_provider passes only the model and api_base, never the caller's provider
        let request = ResolutionRequest {
            model: &model,
            custom_llm_provider: None,
            api_base: keywords.api_base(),
            api_key: None,
        };
        let provider = match resolve(&request, self.resolution) {
            Ok(found) => found.provider.slug().to_owned(),
            Err(source) => {
                return Err(AspeechError::Raised {
                    provider: keywords
                        .custom_llm_provider()
                        .unwrap_or(FALLBACK_PROVIDER)
                        .to_owned(),
                    model,
                    error: Invoker::Error::from(source),
                });
            }
        };
        let raised = |error| AspeechError::Raised {
            model: model.clone(),
            provider: provider.clone(),
            error,
        };
        match self
            .invoker
            .speech(&keywords, context)
            .await
            .map_err(raised)?
        {
            SpeechReturn::Coroutine(pending) => {
                pending.await.map(SpeechReturn::Finished).map_err(raised)
            }
            // LiteLLM discards the finished value and returns a second call's result unchecked
            SpeechReturn::Finished(_) => self
                .invoker
                .speech(&keywords, context)
                .await
                .map_err(raised),
        }
    }
}
