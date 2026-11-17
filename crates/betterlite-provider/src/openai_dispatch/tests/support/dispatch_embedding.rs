use crate::openai_dispatch::call::{Composition, EmbeddingCall, OpenAiOperation};
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::{FixtureTextSource, TextReply};
use crate::operation::OperationResult;

use super::run;

pub(crate) async fn dispatch_embedding(
    composition: &Composition,
    call: EmbeddingCall,
    answer: &FixtureAnswer,
) -> Result<OperationResult, OpenAiProviderError> {
    let text = FixtureTextSource::new(TextReply::new(200, Vec::new()));
    let mut logger = FixtureLogger::default();
    run(
        composition,
        OpenAiOperation::Embedding(Box::new(call)),
        answer,
        &text,
        &mut logger,
    )
    .await
}
