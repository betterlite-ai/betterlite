use betterlite_db::key_attribution::history::rows::SpendLogDigestRow;
use betterlite_db::key_attribution::history::window::{
    HistoryWindow, is_spend_log_digest, spend_log_cache_key,
};
use betterlite_db::key_attribution::metadata::KeyMetadata;
use chrono::NaiveDateTime;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
struct CacheWindow {
    digest: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
}

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    keys: Vec<String>,
    windows: Vec<CacheWindow>,
    rows: Vec<SpendLogDigestRow>,
}

#[derive(serde::Serialize)]
struct Output {
    digests: Vec<bool>,
    cache_keys: Vec<String>,
    metadata: Vec<KeyMetadata>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = Output {
            digests: input
                .keys
                .iter()
                .map(|key| is_spend_log_digest(key))
                .collect(),
            cache_keys: input
                .windows
                .iter()
                .map(|cached| {
                    let window = HistoryWindow {
                        start: cached.start,
                        end: cached.end,
                    };
                    spend_log_cache_key(&cached.digest, &window)
                })
                .collect(),
            metadata: input.rows.iter().map(SpendLogDigestRow::metadata).collect(),
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
