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
    if let Some(facts) = database_facts(failure) {
        tracing::error!(
            entity,
            hint,
            cause = facts.sqlstate,
            table = facts.table,
            column = facts.column,
            constraint = facts.constraint,
            "budget lookup failed for {entity}; the cache will not be populated and each request will reach the database"
        );
        return;
    }
    let cause = redactor.redact_internal_details(&message);
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

// The database error's identifiers, never its message, detail, hint or where text.
struct DatabaseFacts<'error> {
    sqlstate: &'error str,
    table: Option<&'error str>,
    column: Option<&'error str>,
    constraint: Option<&'error str>,
}

fn database_facts<'a>(failure: &'a (dyn Error + 'static)) -> Option<DatabaseFacts<'a>> {
    let mut link = failure;
    loop {
        if let Some(error) = link
            .downcast_ref::<tokio_postgres::Error>()
            .and_then(tokio_postgres::Error::as_db_error)
        {
            return Some(DatabaseFacts {
                sqlstate: error.code().code(),
                table: error.table(),
                column: error.column(),
                constraint: error.constraint(),
            });
        }
        link = link.source()?;
    }
}
