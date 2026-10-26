use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use tracing::warn;

use super::{CbfDataFrame, CbfRecord, CbfValue, CzrnError, CzrnGenerator, UsageRecord};

#[derive(Debug, Clone, Default)]
pub struct CbfTransformer {
    pub czrn_generator: CzrnGenerator,
}

impl CbfTransformer {
    pub fn transform(&self, rows: &[UsageRecord]) -> CbfDataFrame {
        let filtered: Vec<&UsageRecord> = rows
            .iter()
            .filter(|row| row.successful_requests > 0)
            .collect();
        let zero_dropped = rows.len().saturating_sub(filtered.len());
        if zero_dropped > 0 {
            warn!(
                dropped = zero_dropped,
                total = rows.len(),
                "dropped records with zero successful_requests"
            );
        }
        let mut result = Vec::with_capacity(filtered.len());
        let mut invalid = 0_usize;
        for row in filtered {
            match self.create_cbf_record(row) {
                Ok(record) => result.push(record),
                Err(_) => invalid = invalid.saturating_add(1),
            }
        }
        if invalid > 0 {
            warn!(
                dropped = invalid,
                filtered = result.len(),
                "dropped records with invalid CZRNs"
            );
        }
        result
    }

    pub fn create_cbf_record(&self, row: &UsageRecord) -> Result<CbfRecord, CzrnError> {
        let resource_id = self.czrn_generator.create_from_litellm_data(row)?;
        let components = self.czrn_generator.extract_components(&resource_id)?;
        let mut record = required_fields(row, &resource_id, &components);
        add_tags(&mut record, row);
        Ok(record)
    }
}

fn required_fields(row: &UsageRecord, resource_id: &str, components: &[String; 6]) -> CbfRecord {
    let usage_start = parse_date(row.date.as_deref());
    let mut record = CbfRecord::new();
    record.insert(
        "time/usage_start".to_owned(),
        usage_start.map_or(CbfValue::None, CbfValue::DateTime),
    );
    record.insert("cost/cost".to_owned(), CbfValue::Money(row.spend));
    record.insert(
        "resource/id".to_owned(),
        CbfValue::Text(resource_id.to_owned()),
    );
    record.insert(
        "usage/amount".to_owned(),
        CbfValue::Integer(row.prompt_tokens.saturating_add(row.completion_tokens)),
    );
    record.insert(
        "usage/units".to_owned(),
        CbfValue::Text("tokens".to_owned()),
    );
    insert_dimensions(&mut record, row, components);
    record
}

fn insert_dimensions(
    record: &mut BTreeMap<String, CbfValue>,
    row: &UsageRecord,
    components: &[String; 6],
) {
    let account = format_account(row.api_key_alias.as_deref(), &row.api_key);
    let fields = [
        ("resource/service", row.model_group.clone()),
        ("resource/account", account),
        ("resource/region", components[2].clone()),
        ("resource/usage_family", row.custom_llm_provider.clone()),
        ("action/operation", row.team_id.clone()),
        ("lineitem/type", "Usage".to_owned()),
        ("resource/tag:provider", components[1].clone()),
        ("resource/tag:model", components[5].clone()),
    ];
    for (name, value) in fields {
        record.insert(name.to_owned(), CbfValue::Text(value));
    }
}

fn parse_date(value: Option<&str>) -> Option<DateTime<Utc>> {
    let parsed_date =
        value.and_then(|date_text| NaiveDate::parse_from_str(date_text, "%Y-%m-%d").ok());
    if let Some(parsed_day) = parsed_date {
        return parsed_day
            .and_hms_opt(0, 0, 0)
            .map(|midnight| DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc));
    }
    let date_text = value?;
    DateTime::parse_from_rfc3339(date_text)
        .ok()
        .map(|parsed_datetime| parsed_datetime.with_timezone(&Utc))
}

fn format_account(alias: Option<&str>, api_key: &str) -> String {
    let hash: String = api_key.chars().take(8).collect();
    match alias.filter(|value| !value.is_empty()) {
        Some(account_alias) => format!("{account_alias}|{hash}"),
        None => hash,
    }
}

fn add_tags(record: &mut CbfRecord, row: &UsageRecord) {
    add_source_dimensions(record, row);
    add_tag(
        record,
        "prompt_tokens",
        (row.prompt_tokens > 0)
            .then(|| row.prompt_tokens.to_string())
            .as_deref(),
    );
    add_tag(
        record,
        "completion_tokens",
        (row.completion_tokens > 0)
            .then(|| row.completion_tokens.to_string())
            .as_deref(),
    );
}

fn add_source_dimensions(record: &mut CbfRecord, row: &UsageRecord) {
    let api_key_prefix: String = row.api_key.chars().take(8).collect();
    add_identity_tags(record, row, api_key_prefix.as_str());
    add_usage_tags(record, row);
}

fn add_identity_tags(record: &mut CbfRecord, row: &UsageRecord, api_key_prefix: &str) {
    add_tag(record, "entity_type", Some("team"));
    add_tag(
        record,
        "entity_id",
        Some(row.team_alias.as_deref().unwrap_or(&row.team_id)),
    );
    add_tag(
        record,
        "team_alias",
        Some(row.team_alias.as_deref().unwrap_or("unknown")),
    );
    add_tag(record, "model", Some(row.model.as_str()));
    add_tag(record, "model_group", Some(row.model_group.as_str()));
    add_tag(record, "provider", Some(row.custom_llm_provider.as_str()));
    add_tag(record, "api_key_prefix", Some(api_key_prefix));
    add_tag(record, "api_key_alias", row.api_key_alias.as_deref());
    add_tag(record, "user_email", row.user_email.as_deref());
}

fn add_usage_tags(record: &mut CbfRecord, row: &UsageRecord) {
    let api_requests = row.api_requests.to_string();
    add_tag(record, "api_requests", Some(api_requests.as_str()));
    let successful_requests = row.successful_requests.to_string();
    add_tag(
        record,
        "successful_requests",
        Some(successful_requests.as_str()),
    );
    let failed_requests = row.failed_requests.to_string();
    add_tag(record, "failed_requests", Some(failed_requests.as_str()));
    let cache_creation = row.cache_creation_input_tokens.to_string();
    add_tag(
        record,
        "cache_creation_tokens",
        Some(cache_creation.as_str()),
    );
    let cache_read = row.cache_read_input_tokens.to_string();
    add_tag(record, "cache_read_tokens", Some(cache_read.as_str()));
    add_tag(
        record,
        "organization_alias",
        row.organization_alias.as_deref(),
    );
    add_tag(record, "project_alias", row.project_alias.as_deref());
    add_tag(record, "user_alias", row.user_alias.as_deref());
}

fn add_tag(record: &mut CbfRecord, key: &str, tag_value: Option<&str>) {
    let Some(candidate) =
        tag_value.filter(|candidate| !candidate.is_empty() && *candidate != "unknown")
    else {
        return;
    };
    record.insert(
        format!("resource/tag:{key}"),
        CbfValue::Text(candidate.to_owned()),
    );
}

#[path = "tests.rs"]
mod tests;
