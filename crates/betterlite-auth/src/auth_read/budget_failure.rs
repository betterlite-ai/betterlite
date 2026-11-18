use betterlite_redact::secret::Redactor;

const SCHEMA_MARKERS: [&str; 5] = ["column", "schema", "does not exist", "prisma", "migrate"];
const SCHEMA_HINT: &str = " Run `betterlite-db-admin migrate` to fix schema mismatches.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureClass {
    Exception,
    Named,
}

pub struct BudgetLookupFailure<'failure> {
    pub class: FailureClass,
    pub message: &'failure str,
}

pub fn log_budget_lookup_failure(
    redactor: &Redactor,
    entity: &str,
    failure: &BudgetLookupFailure<'_>,
) {
    if failure.class == FailureClass::Exception && failure.message.is_empty() {
        return;
    }
    let lowered = failure.message.to_lowercase();
    let hint = if SCHEMA_MARKERS
        .iter()
        .any(|marker| lowered.contains(*marker))
    {
        SCHEMA_HINT
    } else {
        ""
    };
    let cause = redactor.redact_string(failure.message);
    tracing::error!(
        entity,
        hint,
        cause = %cause,
        "budget lookup failed for {entity}; the cache will not be populated and each request will reach the database"
    );
}
