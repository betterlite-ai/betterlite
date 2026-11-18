use std::error::Error;

use betterlite_redact::secret::Redactor;

const SCHEMA_MARKERS: [&str; 5] = ["column", "schema", "does not exist", "prisma", "migrate"];
const SCHEMA_HINT: &str = " Run `betterlite-db-admin migrate` to fix schema mismatches.";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ExpectedLookupMiss {
    pub message: String,
}

pub fn log_budget_lookup_failure(
    redactor: &Redactor,
    entity: &str,
    failure: &(dyn Error + 'static),
) {
    // Only the bare Exception with an empty message is suppressed, never a subclass.
    if failure.downcast_ref::<ExpectedLookupMiss>().is_some() && failure.to_string().is_empty() {
        return;
    }
    let message = chain_text(failure);
    let lowered = message.to_lowercase();
    let hint = if SCHEMA_MARKERS
        .iter()
        .any(|marker| lowered.contains(*marker))
    {
        SCHEMA_HINT
    } else {
        ""
    };
    let cause = redactor.redact_string(&message);
    tracing::error!(
        entity,
        hint,
        cause = %cause,
        "budget lookup failed for {entity}; the cache will not be populated and each request will reach the database"
    );
}

fn chain_text(failure: &(dyn Error + 'static)) -> String {
    let mut text = failure.to_string();
    let mut source = failure.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
