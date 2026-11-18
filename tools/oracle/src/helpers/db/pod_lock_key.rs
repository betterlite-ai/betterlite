use betterlite_db::queues::pod_lock::owner::lease_key;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cronjob_id: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(serde_json::Value::String(lease_key(&input.cronjob_id)))
    }
}
