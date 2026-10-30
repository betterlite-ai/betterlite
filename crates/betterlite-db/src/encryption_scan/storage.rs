use super::{
    COVERED_TABLE_SPECS, CoveredTableSpec, DbCipher, LocationReport, MigrationJson,
    MigrationReport, ScanError, SecretText, classify_value, decode_migration_json,
};
use futures_util::TryStreamExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use std::{iter, pin::pin, time::Duration};
use tokio::time::timeout;
use tokio_postgres::{GenericClient, Row};

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
    let Ok(rows) = read_table_rows(client, spec).await else {
        return Ok(report);
    };
    let empty = report.clone();
    let mut stream = pin!(rows);
    loop {
        match stream.try_next().await {
            Ok(Some(row)) => scan_row(&mut report, &row, spec, cipher)?,
            Ok(None) => return Ok(report),
            Err(_) => return Ok(empty),
        }
    }
}

async fn read_table_rows<C>(
    client: &C,
    spec: &CoveredTableSpec,
) -> Result<tokio_postgres::RowStream, ScanError>
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
    let sql = format!("SELECT {columns} FROM \"{}\"", spec.table);
    client
        .query_raw(&sql, iter::empty::<&str>())
        .await
        .map_err(|source| ScanError::Read {
            location: spec.location.to_owned(),
            source,
        })
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

async fn scan_config_env_vars<C>(client: &C, cipher: &DbCipher) -> Result<LocationReport, ScanError>
where
    C: GenericClient + Sync,
{
    let mut report = LocationReport {
        location: String::from("config_environment_variables"),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    };
    let Ok(Some(row)) = client
        .query_opt(
            "SELECT \"param_value\"::text FROM \"LiteLLM_Config\" WHERE \"param_name\" = $1",
            &[&"environment_variables"],
        )
        .await
    else {
        return Ok(report);
    };
    if let Some(value) = row_value(&row, "param_value")? {
        match value {
            MigrationJson::String(text) => {
                if let Ok(decoded) = decode_migration_json(text.0.expose_secret().as_bytes()) {
                    scan_json_value(&mut report, &decoded, cipher)?;
                }
            }
            other @ (MigrationJson::Null
            | MigrationJson::Bool(_)
            | MigrationJson::Number(_)
            | MigrationJson::Array(_)
            | MigrationJson::Object(_)) => scan_json_value(&mut report, &other, cipher)?,
        }
    }
    Ok(report)
}

async fn scan_tables<C>(client: &C, cipher: &DbCipher) -> Result<MigrationReport, ScanError>
where
    C: GenericClient + Sync,
{
    let mut locations = Vec::new();
    for spec in COVERED_TABLE_SPECS {
        locations.push(scan_one_table(client, spec, cipher).await?);
    }
    locations.push(scan_config_env_vars(client, cipher).await?);
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

pub async fn _scan_covered_tables<C>(
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
