use super::{
    COVERED_TABLE_SPECS, CoveredTableSpec, DbCipher, LocationReport, MigrationJson,
    MigrationReport, ScanError, SecretText, classify_value,
};
use tokio_postgres::{GenericClient, types::Json};

fn row_value(row: &tokio_postgres::Row, column: &str) -> Option<MigrationJson> {
    if let Ok(Json(value)) = row.try_get::<_, Json<MigrationJson>>(column) {
        return Some(value);
    }
    let value = row.try_get::<_, String>(column).ok()?;
    let parsed = serde_json::from_str(&value).ok();
    parsed.or_else(|| Some(MigrationJson::String(SecretText::new(value))))
}

async fn scan_one_table<C>(
    client: &C,
    spec: &CoveredTableSpec,
    cipher: &DbCipher,
) -> Result<LocationReport, ScanError>
where
    C: GenericClient + Sync,
{
    let columns = spec
        .json_columns
        .iter()
        .chain(spec.scalar_columns.iter())
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {columns} FROM \"{}\"", spec.table);
    let rows = match client.query(&sql, &[]).await {
        Ok(rows) => rows,
        Err(source) => {
            return Err(ScanError::Read {
                location: spec.location.to_owned(),
                source,
            });
        }
    };
    let mut report = LocationReport {
        location: spec.location.to_owned(),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    };
    for row in &rows {
        scan_row(&mut report, row, spec, cipher)?;
    }
    Ok(report)
}

fn scan_row(
    report: &mut LocationReport,
    row: &tokio_postgres::Row,
    spec: &CoveredTableSpec,
    cipher: &DbCipher,
) -> Result<(), ScanError> {
    for column in spec.json_columns {
        let Some(value) = row_value(row, column) else {
            continue;
        };
        if spec.table == "litellm_mcpservertable"
            && (*column == "static_headers" || *column == "env")
        {
            report.count(super::classify_secret_map(&value, cipher, column))?;
        } else {
            scan_json_value(report, &value, cipher)?;
        }
    }
    for column in spec.scalar_columns {
        let Some(MigrationJson::String(text)) = row_value(row, column) else {
            continue;
        };
        report.count(Some(classify_value(
            &MigrationJson::String(text),
            cipher,
            column,
        )))?;
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
    let row = match client
        .query_opt(
            "SELECT \"param_value\" FROM \"litellm_config\" WHERE \"param_name\" = $1",
            &[&"environment_variables"],
        )
        .await
    {
        Ok(row) => row,
        Err(source) => {
            return Err(ScanError::Read {
                location: report.location.clone(),
                source,
            });
        }
    };
    if let Some(record) = row
        && let Some(value) = row_value(&record, "param_value")
    {
        for text in super::iter_encrypted_strings(&value) {
            report.count(Some(classify_value(
                &MigrationJson::String(text.clone()),
                cipher,
                "scan",
            )))?;
        }
    }
    Ok(report)
}

pub async fn _scan_covered_tables<C>(client: &C, cipher: &DbCipher) -> MigrationReport
where
    C: GenericClient + Sync,
{
    let mut locations = Vec::new();
    for spec in COVERED_TABLE_SPECS {
        locations.push(
            scan_one_table(client, spec, cipher)
                .await
                .unwrap_or_else(|_| empty_report(spec.location)),
        );
    }
    locations.push(
        scan_config_env_vars(client, cipher)
            .await
            .unwrap_or_else(|_| empty_report("config_environment_variables")),
    );
    MigrationReport { locations }
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

fn empty_report(location: &str) -> LocationReport {
    let zero = 0;
    LocationReport {
        location: location.to_owned(),
        scanned: zero,
        migrated: zero,
        already_v2: zero,
        plaintext: zero,
        undecryptable: zero,
        legacy: zero,
    }
}
