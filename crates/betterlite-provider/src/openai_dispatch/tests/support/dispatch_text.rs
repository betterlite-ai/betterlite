use crate::openai_dispatch::call::{Composition, OpenAiOperation, TextCall};
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::FixtureTextSource;
use crate::operation::OperationResult;

use super::run;

pub(crate) async fn dispatch_text(
    composition: &Composition,
    call: TextCall,
    text: &FixtureTextSource,
    logger: &mut FixtureLogger,
) -> Result<OperationResult, OpenAiProviderError> {
    let answer = FixtureAnswer::new(200, Vec::new());
    run(
        composition,
        OpenAiOperation::Text(Box::new(call)),
        &answer,
        text,
        logger,
    )
    .await
}
