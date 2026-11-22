use std::error::Error;
use std::mem;

use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_guardrails::unified_dispatch::flags::is_redundant_scan;

use super::block_only::{Scanner, Window};
use crate::guardrails::unified_stream::StreamOutput;
use crate::guardrails::unified_stream::round::call_streaming;

// The withheld chunks are copies the scan never rewrites, so the originals go out whatever release_on_scan says
pub(crate) async fn finish_stream<T, G, E>(window: &mut Window<E>, scanner: &mut Scanner<'_, T, G>)
where
    T: BaseTranslation<Guardrail = G, LogTarget = GuardrailLogTarget, Failure = GuardrailFailure>
        + Sync,
    G: GuardrailContract,
    E: Error + Send + Sync + 'static,
{
    let name = scanner.guardrail_name;
    tracing::debug!(
        "Processing final streaming response with all {} chunks for guardrail {name}",
        window.responses_so_far.len()
    );
    let buffered = mem::take(&mut window.withheld_items);
    let end_key = scanner
        .params
        .translation
        .get_streaming_scan_key(&window.responses_so_far);
    if is_redundant_scan(end_key.as_ref(), window.last_scan_key.as_ref()) {
        tracing::debug!(
            "Skipping end-of-stream scan for guardrail {name}: the last sampled round already scanned it all"
        );
    } else {
        let outcome = call_streaming(&mut scanner.params, &mut window.responses_so_far, None).await;
        if let Err(failure) = outcome {
            window.terminate(&scanner.params, failure);
            return;
        }
    }
    for item in buffered {
        window.outputs.push_back(Ok(StreamOutput::Item(item)));
    }
    for item in mem::take(&mut window.pending_items) {
        window.send(item);
    }
}
