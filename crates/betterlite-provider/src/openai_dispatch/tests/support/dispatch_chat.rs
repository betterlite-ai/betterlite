use crate::openai_dispatch::call::{ChatCall, Composition, OpenAiOperation};
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::{FixtureTextSource, TextReply};
use crate::operation::OperationResult;

use super::run;

pub(crate) async fn dispatch_chat(
    composition: &Composition,
    call: ChatCall,
    answer: &FixtureAnswer,
    logger: &mut FixtureLogger,
) -> Result<OperationResult, OpenAiProviderError> {
    let text = FixtureTextSource::new(TextReply::new(200, Vec::new()));
    run(
        composition,
        OpenAiOperation::Chat(Box::new(call)),
        answer,
        &text,
        logger,
    )
    .await
}
