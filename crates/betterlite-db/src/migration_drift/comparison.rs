use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::database_tls::translation::{TranslateError, strip_prisma_query_params};
use crate::migration_ledger::connection::{ConnectionError, connect};
use crate::schema_contract::comparison::Comparison;
use crate::schema_contract::{ContractError, Fact, SchemaContract};

use super::MigrationDriftCheck;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaCheck {
    UpdatesEnabled,
    Unavailable,
    InSync,
    Differing(Comparison),
}

#[derive(Debug, Error)]
pub enum SchemaReadError {
    #[error("DATABASE_URL is not set, so the schema comparison cannot run")]
    MissingUrl,
    #[error("the database URL could not be normalized for the schema comparison")]
    Url(#[from] TranslateError),
    #[error("the database connection for the schema comparison could not be opened")]
    Connect(#[from] ConnectionError),
    #[error("the database schema could not be inspected")]
    Inspect(#[from] ContractError),
}

// Mirrors check_prisma_schema_diff: a mismatch is a warning, never a startup failure
#[must_use]
pub fn evaluate(expected: &SchemaContract, live: &SchemaContract) -> SchemaCheck {
    let comparison = expected.compare(live);
    if comparison.differences.is_empty() {
        return SchemaCheck::InSync;
    }
    tracing::warn!(
        difference_count = comparison.differences.len(),
        differences = %summary(&comparison),
        "the database schema is out of sync with this build's schema; schema updates are disabled, so startup continues"
    );
    SchemaCheck::Differing(comparison)
}

#[must_use]
pub fn summary(comparison: &Comparison) -> String {
    let shown: Vec<String> = comparison
        .differences
        .iter()
        .take(5)
        .map(|(fact, change)| format!("{} ({:?} {change:?})", fact_name(fact), fact.attribute))
        .collect();
    let rest = comparison.differences.len().saturating_sub(shown.len());
    if rest == 0 {
        shown.join(", ")
    } else {
        format!("{}, and {rest} more", shown.join(", "))
    }
}

fn fact_name(fact: &Fact) -> String {
    if fact.member.is_empty() {
        fact.relation.clone()
    } else {
        format!("{}.{}", fact.relation, fact.member)
    }
}

// The disabled-update comparison: every failure is a warning, so startup is never prevented
pub async fn check_schema(
    check: &MigrationDriftCheck,
    updates_disabled: bool,
    expected: Option<&SchemaContract>,
    schema: &str,
    read_timeout: Duration,
) -> SchemaCheck {
    if !updates_disabled {
        return SchemaCheck::UpdatesEnabled;
    }
    let Some(contract) = expected else {
        tracing::warn!(
            "this build's schema contract is unavailable, so schema drift was not checked; startup continues"
        );
        return SchemaCheck::Unavailable;
    };
    match live_contract(check, schema, read_timeout).await {
        Ok(live) => evaluate(contract, &live),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "the database schema could not be compared with this build's schema; startup continues"
            );
            SchemaCheck::Unavailable
        }
    }
}

async fn live_contract(
    check: &MigrationDriftCheck,
    schema: &str,
    read_timeout: Duration,
) -> Result<SchemaContract, SchemaReadError> {
    let Some(database_url) = &check.database_url else {
        return Err(SchemaReadError::MissingUrl);
    };
    let stripped = strip_prisma_query_params(database_url.expose_secret())?;
    let client = connect(&SecretString::from(stripped)).await?;
    SchemaContract::read(&*client, schema, read_timeout)
        .await
        .map_err(SchemaReadError::Inspect)
}
