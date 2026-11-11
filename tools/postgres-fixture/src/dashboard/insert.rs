use secrecy::ExposeSecret as _;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::payload::JsonPayload;
use super::{DashboardFixtureError, SpendLogSeed};
use crate::seed::FixtureSeed;

const COLUMNS: &str = "\"request_id\",\"call_type\",\"api_key\",\"spend\",\"total_tokens\",\
\"prompt_tokens\",\"completion_tokens\",\"startTime\",\"endTime\",\"completionStartTime\",\
\"request_duration_ms\",\"model\",\"model_id\",\"model_group\",\"custom_llm_provider\",\"user\",\
\"team_id\",\"end_user\",\"session_id\",\"status\",\"cache_hit\",\"litellm_call_id\",\"metadata\",\
\"request_tags\",\"messages\",\"response\",\"proxy_server_request\"";

const TIMESTAMP_CAST: &str = "::text::timestamp(3)";
const SPEND_CAST: &str = "::text::numeric::double precision";
const JSON_CAST: &str = "::text::jsonb";

struct Bindings<'log> {
    placeholders: Vec<String>,
    params: Vec<&'log (dyn ToSql + Sync)>,
}

impl<'log> Bindings<'log> {
    fn bind<T>(&mut self, value: &'log T, cast: &str)
    where
        T: ToSql + Sync,
    {
        self.params.push(value);
        let position = self.params.len();
        self.placeholders.push(format!("${position}{cast}"));
    }

    // an omitted model or payload keeps the value the pinned schema defaults it to
    fn bind_or_default(&mut self, value: Option<&'log String>, cast: &str) {
        match value {
            Some(text) => self.bind(text, cast),
            None => self.placeholders.push("DEFAULT".to_owned()),
        }
    }
}

pub(super) async fn insert_log(
    client: &Client,
    row: usize,
    log: &SpendLogSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardFixtureError> {
    let key = seed
        .keys
        .get(log.key_index)
        .ok_or(DashboardFixtureError::UnknownKey {
            row,
            key_index: log.key_index,
            keys: seed.keys.len(),
        })?;
    let api_key = FixtureSeed::key_hash(key.token.expose_secret());
    let spend = log.spend.to_string();
    let payloads = [
        serialize_payload(log.metadata.as_ref())?,
        serialize_payload(log.request_tags.as_ref())?,
        serialize_payload(log.messages.as_ref())?,
        serialize_payload(log.response.as_ref())?,
        serialize_payload(log.proxy_server_request.as_ref())?,
    ];
    let mut bindings = Bindings {
        placeholders: Vec::new(),
        params: Vec::new(),
    };
    bind_log(&mut bindings, log, &api_key, &spend, &payloads);
    let statement = format!(
        "INSERT INTO \"LiteLLM_SpendLogs\" ({COLUMNS}) VALUES ({})",
        bindings.placeholders.join(",")
    );
    client
        .execute(&statement, &bindings.params)
        .await
        .map_err(|source| DashboardFixtureError::Insert { row, source })?;
    Ok(())
}

fn serialize_payload(
    payload: Option<&JsonPayload>,
) -> Result<Option<String>, DashboardFixtureError> {
    payload
        .map(serde_json::to_string)
        .transpose()
        .map_err(|source| DashboardFixtureError::PayloadUnserializable { source })
}

fn bind_log<'log>(
    bindings: &mut Bindings<'log>,
    log: &'log SpendLogSeed,
    api_key: &'log String,
    spend: &'log String,
    payloads: &'log [Option<String>; 5],
) {
    bindings.bind(&log.request_id, "");
    bindings.bind(&log.call_type, "");
    bindings.bind(api_key, "");
    bindings.bind(spend, SPEND_CAST);
    bindings.bind(&log.total_tokens, "");
    bindings.bind(&log.prompt_tokens, "");
    bindings.bind(&log.completion_tokens, "");
    bindings.bind(&log.start_time, TIMESTAMP_CAST);
    bindings.bind(&log.end_time, TIMESTAMP_CAST);
    bindings.bind(&log.completion_start_time, TIMESTAMP_CAST);
    bindings.bind(&log.request_duration_ms, "");
    bindings.bind_or_default(log.model.as_ref(), "");
    bindings.bind(&log.model_id, "");
    bindings.bind(&log.model_group, "");
    bindings.bind(&log.custom_llm_provider, "");
    bindings.bind(&log.user, "");
    bindings.bind(&log.team_id, "");
    bindings.bind(&log.end_user, "");
    bindings.bind(&log.session_id, "");
    bindings.bind(&log.status, "");
    bindings.bind(&log.cache_hit, "");
    bindings.bind(&log.litellm_call_id, "");
    for payload in payloads {
        bindings.bind_or_default(payload.as_ref(), JSON_CAST);
    }
}
