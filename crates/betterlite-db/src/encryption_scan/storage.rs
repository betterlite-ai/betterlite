use super::{
    COVERED_TABLE_SPECS, CoveredTableSpec, DbCipher, LocationReport, MigrationJson,
    MigrationReport, ScanError, SecretText, classify_value, decode_migration_json,
};
use futures_util::TryStreamExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use std::{iter, pin::pin, time::Duration};
use tokio::time::timeout;
use tokio_postgres::{GenericClient, Row};

const MARK_SAVEPOINT: &str = "SAVEPOINT betterlite_encryption_scan";
const RECOVER_SAVEPOINT: &str = "ROLLBACK TO SAVEPOINT betterlite_encryption_scan";

fn row_value(row: &Row, column: &str) -> Result<Option<MigrationJson>, ScanError> {
    let raw: Option<String> = row.try_get(column).map_err(|source| ScanError::Read {
        location: column.to_owned(),
        source,
    })?;
    raw.map(|text| {
        let secret = SecretString::from(text);
        decode_migration_json(secret.expose_secret().as_bytes())
    })
    .transpose()
}

async fn scan_one_table<C>(
    client: &C,
    spec: &CoveredTableSpec,
    cipher: &DbCipher,
) -> Result<LocationReport, ScanError>
where
    C: GenericClient + Sync,
{
    let mut report = LocationReport {
        location: spec.location.to_owned(),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    };
    let Some(rows) = read_table_rows(client, spec).await? else {
        return Ok(report);
    };
    let empty = report.clone();
    let mut stream = pin!(rows);
    loop {
        match stream.try_next().await {
            Ok(Some(row)) => scan_row(&mut report, &row, spec, cipher)?,
            Ok(None) => return Ok(report),
            Err(source) => {
                recover_unavailable_location(client, spec.location, &source).await;
                return Ok(empty);
            }
        }
    }
}

async fn recover_unavailable_location<C>(client: &C, location: &str, source: &tokio_postgres::Error)
where
    C: GenericClient + Sync,
{
    tracing::debug!(location, error = %source, "scan: location unavailable");
    drop(client.execute(RECOVER_SAVEPOINT, &[]).await);
}

async fn read_table_rows<C>(
    client: &C,
    spec: &CoveredTableSpec,
) -> Result<Option<tokio_postgres::RowStream>, ScanError>
where
    C: GenericClient + Sync,
{
    let columns = spec
        .json_columns
        .iter()
        .chain(spec.scalar_columns.iter())
        .map(|column| format!("\"{column}\"::text"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("SELECT {columns} FROM \"{}\"", spec.table);
    if spec.table == "LiteLLM_Config" {
        sql.push_str(" WHERE \"param_name\" = 'environment_variables'");
    }
    drop(client.execute(MARK_SAVEPOINT, &[]).await);
    match client.query_raw(&sql, iter::empty::<&str>()).await {
        Ok(rows) => Ok(Some(rows)),
        Err(source) => {
            recover_unavailable_location(client, spec.location, &source).await;
            Ok(None)
        }
    }
}

fn scan_row(
    report: &mut LocationReport,
    row: &Row,
    spec: &CoveredTableSpec,
    cipher: &DbCipher,
) -> Result<(), ScanError> {
    for column in spec.json_columns {
        let Some(value) = row_value(row, column)? else {
            continue;
        };
        if spec.table == "LiteLLM_MCPServerTable"
            && (*column == "static_headers" || *column == "env")
        {
            report.count(super::classify_secret_map(&value, cipher, column))?;
        } else {
            let decoded = match &value {
                MigrationJson::String(text) => {
                    decode_migration_json(text.0.expose_secret().as_bytes()).ok()
                }
                MigrationJson::Null
                | MigrationJson::Bool(_)
                | MigrationJson::Number(_)
                | MigrationJson::Array(_)
                | MigrationJson::Object(_) => None,
            };
            if spec.table == "LiteLLM_Config"
                && matches!(value, MigrationJson::String(_))
                && decoded.is_none()
            {
                continue;
            }
            scan_json_value(report, decoded.as_ref().unwrap_or(&value), cipher)?;
        }
    }
    for column in spec.scalar_columns {
        if let Ok(Some(text)) = row.try_get::<_, Option<String>>(*column) {
            let value = MigrationJson::String(SecretText(SecretString::from(text)));
            report.count(Some(classify_value(&value, cipher, column)))?;
        }
    }
    Ok(())
}

async fn scan_tables<C>(client: &C, cipher: &DbCipher) -> Result<MigrationReport, ScanError>
where
    C: GenericClient + Sync,
{
    let mut locations = Vec::new();
    let config = CoveredTableSpec {
        location: "config_environment_variables",
        table: "LiteLLM_Config",
        json_columns: &["param_value"],
        scalar_columns: &[],
    };
    for spec in COVERED_TABLE_SPECS.iter().chain(iter::once(&config)) {
        locations.push(scan_one_table(client, spec, cipher).await?);
    }
    Ok(MigrationReport { locations })
}

fn scan_json_value(
    report: &mut LocationReport,
    value: &MigrationJson,
    cipher: &DbCipher,
) -> Result<(), ScanError> {
    for text in super::iter_encrypted_strings(value) {
        report.count(Some(classify_value(
            &MigrationJson::String(text.clone()),
            cipher,
            "scan",
        )))?;
    }
    Ok(())
}

mod tests;

pub async fn scan_covered_tables<C>(
    client: &C,
    cipher: &DbCipher,
    deadline: Duration,
) -> Result<MigrationReport, ScanError>
where
    C: GenericClient + Sync,
{
    let scan = scan_tables(client, cipher);
    timeout(deadline, scan).await.map_err(ScanError::Timeout)?
}
