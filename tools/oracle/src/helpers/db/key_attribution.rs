use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_attribution::metadata::{
    KeyMetadata, UserDetails, attach_details, cli_candidates, cli_metadata, user_details,
};
use betterlite_db::key_attribution::projection::{ExportFields, missing_keys, project_rows};
use betterlite_db::key_attribution::rows::{
    DailyOwnerRow, TokenDigestRow, daily_owners, digest_keys, token_metadata, unresolved_digests,
};
use betterlite_models::key_digest::is_valid_sha256_hash;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    digest_values: Vec<String>,
    cli_keys: BTreeSet<String>,
    double_hashed_keys: BTreeSet<String>,
    daily_keys: BTreeSet<String>,
    users: Vec<LiteLLMUserTable>,
    active_token_rows: Vec<TokenDigestRow>,
    deleted_token_rows: Vec<TokenDigestRow>,
    daily_owner_rows: Vec<DailyOwnerRow>,
    attach_metadata: BTreeMap<String, KeyMetadata>,
    export_rows: Vec<BTreeMap<String, JsonData>>,
    #[serde(default)]
    export_fields: ExportFields,
}

#[derive(serde::Serialize)]
struct Output {
    digest: Vec<bool>,
    cli: BTreeMap<String, KeyMetadata>,
    double_hashed: BTreeMap<String, KeyMetadata>,
    daily_owners: BTreeMap<String, String>,
    attached: BTreeMap<String, KeyMetadata>,
    export: Vec<BTreeMap<String, JsonData>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let details = user_details(&input.users);
        let mut attached = input.attach_metadata.clone();
        attach_details(&mut attached, &details);
        let output = Output {
            digest: digest_flags(input),
            cli: cli_metadata(&cli_candidates(&input.cli_keys), &details),
            double_hashed: double_hashed_metadata(
                &input.active_token_rows,
                &input.deleted_token_rows,
                &input.double_hashed_keys,
            ),
            daily_owners: daily_owners(&input.daily_owner_rows, &input.daily_keys),
            attached,
            export: fill_export(input, &details),
        };
        serde_json::to_value(output).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn digest_flags(input: &Input) -> Vec<bool> {
    input
        .digest_values
        .iter()
        .map(|value| is_valid_sha256_hash(value))
        .collect()
}

fn double_hashed_metadata(
    active: &[TokenDigestRow],
    deleted: &[TokenDigestRow],
    keys: &BTreeSet<String>,
) -> BTreeMap<String, KeyMetadata> {
    let digests = digest_keys(keys);
    let from_active = token_metadata(active, &digests);
    let still_missing = unresolved_digests(&from_active, &digests);
    let from_deleted = token_metadata(deleted, &still_missing);
    from_active.into_iter().chain(from_deleted).collect()
}

fn fill_export(
    input: &Input,
    details: &BTreeMap<String, UserDetails>,
) -> Vec<BTreeMap<String, JsonData>> {
    let missing = missing_keys(&input.export_rows, &input.export_fields);
    if missing.is_empty() {
        return input.export_rows.clone();
    }
    let from_session = cli_metadata(&cli_candidates(&missing), details);
    let known: BTreeSet<String> = from_session.keys().cloned().collect();
    let remaining: BTreeSet<String> = missing.difference(&known).cloned().collect();
    let from_double = double_hashed_metadata(
        &input.active_token_rows,
        &input.deleted_token_rows,
        &remaining,
    );
    let mut recovered = from_session;
    recovered.extend(from_double);
    attach_details(&mut recovered, details);
    if recovered.is_empty() {
        return input.export_rows.clone();
    }
    project_rows(&input.export_rows, &recovered, &input.export_fields)
}
