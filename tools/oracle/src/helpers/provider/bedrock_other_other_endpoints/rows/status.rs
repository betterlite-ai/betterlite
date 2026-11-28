use betterlite_bedrock::transcription::error::CallError;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::{
    BedrockOtherOperation, EmbeddingStatusOperation,
};
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use betterlite_provider::bedrock_other_dispatch::test_support::{NOW_SECONDS, instant};
use serde_json::{Value, json};

use super::super::harness::{Harness, Outcome, exchange};
use super::super::model::{Failed, Probe, Unavailable, param_map};
use super::row;
use crate::openai_media_dispatch::failure;

pub(super) async fn status_row(probe: &Probe) -> Result<Value, Failed> {
    let mut params = param_map(probe)?;
    let invocation = probe
        .invocation_arn
        .as_deref()
        .ok_or_else(|| failure("Arn", "a status probe carries an invocation arn"))?;
    let region = probe.region.as_deref().unwrap_or("us-east-1");
    let operation = BedrockOtherOperation::EmbeddingStatus(EmbeddingStatusOperation {
        invocation_arn: invocation,
        region,
        optional_params: &mut params,
        environment: &probe.environment,
        now: instant(NOW_SECONDS).map_err(|error| failure("Instant", error))?,
    });
    let harness = Harness::new(probe)?;
    let outcome: Outcome<CallError> =
        exchange::<Unavailable, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::EmbeddingStatus(body)) => json!({
            "body": body.as_ref(),
            "regions": &outcome.regions,
        }),
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, None)
}
