use super::logger::DatadogLogger;
use super::payloads::{SyncCall, sync_payload};
use super::send::send_plain;

pub async fn log_success_event(logger: &DatadogLogger, call: &SyncCall<'_>) {
    let generated = (logger.ids)();
    let payload = match sync_payload(
        call,
        &logger.settings.environment,
        &logger.tracer,
        logger.settings.use_v1,
        &generated,
    ) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(%error, "Datadog Layer Error");
            return;
        }
    };
    if let Err(error) = send_plain(logger, &payload).await {
        tracing::error!(%error, "Datadog Layer Error");
    }
}
