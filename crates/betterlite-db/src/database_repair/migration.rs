use std::io;
use std::path::Path;

use serde_json::Error as JsonError;
use tokio::io::AsyncReadExt as _;
use tokio_postgres::Client;
use tokio_postgres::Error as PostgresError;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::JsonData;

use super::cells::{RowError, VerificationTokenRow};
use super::{DecodeError, MAX_CSV_BYTES, TypedValue, decode_verification_csv};

const EXISTS_SQL: &str = r#"SELECT 1 FROM "LiteLLM_VerificationToken" WHERE "token" = $1::text"#;

const INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (
    "token", "key_name", "key_alias", "soft_budget_cooldown", "spend", "expires", "models", "aliases",
    "config", "user_id", "team_id", "permissions", "max_parallel_requests", "metadata", "tpm_limit",
    "rpm_limit", "max_budget", "budget_duration", "budget_reset_at", "allowed_cache_controls",
    "model_spend", "model_max_budget", "budget_id", "blocked", "created_at", "updated_at",
    "allowed_routes", "object_permission_id", "created_by", "updated_by", "organization_id"
) VALUES (
    $1::text, $2::text, $3::text, COALESCE($4::bool, false),
    COALESCE($5::numeric::double precision, 0.0::double precision), $6::timestamp(3), $7::text[],
    COALESCE($8::text::jsonb, '{}'::jsonb), COALESCE($9::text::jsonb, '{}'::jsonb),
    $10::text, $11::text, COALESCE($12::text::jsonb, '{}'::jsonb),
    $13::bigint::integer, COALESCE($14::text::jsonb, '{}'::jsonb), $15::bigint, $16::bigint,
    $17::numeric::double precision, $18::text, $19::timestamp(3),
    COALESCE($20::text[], ARRAY[]::TEXT[]), COALESCE($21::text::jsonb, '{}'::jsonb),
    COALESCE($22::text::jsonb, '{}'::jsonb), $23::text, $24::bool,
    COALESCE($25::timestamp(3), LOCALTIMESTAMP),
    COALESCE($26::timestamp(3), LOCALTIMESTAMP),
    COALESCE($27::text[], ARRAY[]::TEXT[]), $28::text, $29::text, $30::text, $31::text
)"#;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("the verification-token CSV could not be read: {0}")]
    Read(#[source] io::Error),
    #[error("the verification-token CSV is not valid: {0}")]
    Decode(#[source] DecodeError),
}

#[derive(Debug, thiserror::Error)]
pub enum ImportFailure {
    #[error("{0}")]
    Decode(#[source] Box<RowError>),
    #[error("the row's JSON values could not be encoded: {0}")]
    Encode(#[source] JsonError),
    #[error("the existing-token lookup failed: {0}")]
    Check(#[source] PostgresError),
    #[error("the token insert failed: {0}")]
    Insert(#[source] PostgresError),
}

#[derive(Debug)]
pub enum ImportEvent {
    Migrated {
        token: String,
    },
    Skipped {
        token: String,
    },
    Failed {
        token: Option<String>,
        source: ImportFailure,
    },
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub events: Vec<ImportEvent>,
}

impl ImportReport {
    #[must_use]
    pub fn migrated(&self) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event, ImportEvent::Migrated { .. }))
            .count()
    }

    #[must_use]
    pub fn skipped(&self) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event, ImportEvent::Skipped { .. }))
            .count()
    }

    #[must_use]
    pub fn failed(&self) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event, ImportEvent::Failed { .. }))
            .count()
    }
}

pub(super) async fn read_verification_csv(path: &Path) -> Result<Vec<u8>, ImportError> {
    let limit = u64::try_from(MAX_CSV_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let file = tokio::fs::File::open(path)
        .await
        .map_err(ImportError::Read)?;
    let mut limited = file.take(limit);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .await
        .map_err(ImportError::Read)?;
    Ok(bytes)
}

pub async fn import_verification_tokens(
    client: &Client,
    path: &Path,
) -> Result<ImportReport, ImportError> {
    let bytes = read_verification_csv(path).await?;
    let decoded = decode_verification_csv(&bytes).map_err(ImportError::Decode)?;
    Ok(migrate(client, decoded).await)
}

async fn migrate(client: &Client, decoded: TypedValue) -> ImportReport {
    let mut report = ImportReport::default();
    for row in decoded.rows {
        match row.outcome {
            Err(source) => report.events.push(ImportEvent::Failed {
                token: row.token,
                source: ImportFailure::Decode(Box::new(source)),
            }),
            Ok(token_row) => import_row(client, token_row, &mut report).await,
        }
    }
    report
}

async fn import_row(client: &Client, row: VerificationTokenRow, report: &mut ImportReport) {
    let token = row.token.clone();
    let existing = client.query_opt(EXISTS_SQL, &[&token]).await;
    let exists = match existing {
        Ok(exists) => exists.is_some(),
        Err(source) => {
            report.events.push(ImportEvent::Failed {
                token: Some(token),
                source: ImportFailure::Check(source),
            });
            return;
        }
    };
    if exists {
        report.events.push(ImportEvent::Skipped { token });
        return;
    }
    match insert_row(client, &row).await {
        Ok(()) => report.events.push(ImportEvent::Migrated { token }),
        Err(source) => report.events.push(ImportEvent::Failed {
            token: Some(token),
            source,
        }),
    }
}

async fn insert_row(client: &Client, row: &VerificationTokenRow) -> Result<(), ImportFailure> {
    let json = JsonParameters::encode(row)?;
    let parameters = insert_parameters(row, &json);
    client
        .execute(INSERT_SQL, &parameters)
        .await
        .map_err(ImportFailure::Insert)?;
    Ok(())
}

struct JsonParameters {
    aliases: Option<String>,
    config: Option<String>,
    permissions: Option<String>,
    metadata: Option<String>,
    model_spend: Option<String>,
    model_max_budget: Option<String>,
}

impl JsonParameters {
    fn encode(row: &VerificationTokenRow) -> Result<Self, ImportFailure> {
        Ok(Self {
            aliases: encode_json(row.aliases.as_ref())?,
            config: encode_json(row.config.as_ref())?,
            permissions: encode_json(row.permissions.as_ref())?,
            metadata: encode_json(row.metadata.as_ref())?,
            model_spend: encode_json(row.model_spend.as_ref())?,
            model_max_budget: encode_json(row.model_max_budget.as_ref())?,
        })
    }
}

fn encode_json(value: Option<&JsonData>) -> Result<Option<String>, ImportFailure> {
    value
        .map(serde_json::to_string)
        .transpose()
        .map_err(ImportFailure::Encode)
}

fn insert_parameters<'row>(
    row: &'row VerificationTokenRow,
    json: &'row JsonParameters,
) -> [&'row (dyn ToSql + Sync); 31] {
    [
        &row.token,
        &row.key_name,
        &row.key_alias,
        &row.soft_budget_cooldown,
        &row.spend,
        &row.expires,
        &row.models,
        &json.aliases,
        &json.config,
        &row.user_id,
        &row.team_id,
        &json.permissions,
        &row.max_parallel_requests,
        &json.metadata,
        &row.tpm_limit,
        &row.rpm_limit,
        &row.max_budget,
        &row.budget_duration,
        &row.budget_reset_at,
        &row.allowed_cache_controls,
        &json.model_spend,
        &json.model_max_budget,
        &row.budget_id,
        &row.blocked,
        &row.created_at,
        &row.updated_at,
        &row.allowed_routes,
        &row.object_permission_id,
        &row.created_by,
        &row.updated_by,
        &row.organization_id,
    ]
}
