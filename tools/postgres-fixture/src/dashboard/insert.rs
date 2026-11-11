use secrecy::ExposeSecret as _;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::{DashboardFixtureError, SpendLogSeed};
use crate::seed::FixtureSeed;

const COLUMNS: &str = "\"request_id\",\"call_type\",\"api_key\",\"spend\",\"total_tokens\",\
\"prompt_tokens\",\"completion_tokens\",\"startTime\",\"endTime\",\"completionStartTime\",\
\"request_duration_ms\",\"model\",\"model_id\",\"model_group\",\"custom_llm_provider\",\"user\",\
\"team_id\",\"end_user\",\"session_id\",\"status\",\"cache_hit\",\"litellm_call_id\",\"metadata\",\
\"request_tags\",\"messages\",\"response\",\"proxy_server_request\"";

const TIMESTAMP_CAST: &str = "::text::timestamp(3)";
const SPEND_CAST: &str = "::text::numeric::double precision";
const JSON_CAST: &str = "::jsonb";

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

    // an omitted payload column keeps the value the pinned schema defaults it to
    fn bind_json<T>(&mut self, value: Option<&'log T>)
    where
        T: ToSql + Sync,
    {
        match value {
            Some(json) => self.bind(json, JSON_CAST),
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
    let mut bindings = Bindings {
        placeholders: Vec::new(),
        params: Vec::new(),
    };
    bind_log(&mut bindings, log, &api_key, &spend);
    let statement = format!(
        "INSERT INTO \"LiteLLM_SpendLogs\" ({COLUMNS}) VALUES ({})",
        bindings.placeholders.join(",")
    );
    client
        .execute(&statement, &bindings.params)
        .await
        .map_err(DashboardFixtureError::Database)?;
    Ok(())
}

fn bind_log<'log>(
    bindings: &mut Bindings<'log>,
    log: &'log SpendLogSeed,
    api_key: &'log String,
    spend: &'log String,
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
    bindings.bind(&log.model, "");
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
    bindings.bind_json(log.metadata.as_ref());
    bindings.bind_json(log.request_tags.as_ref());
    bindings.bind_json(log.messages.as_ref());
    bindings.bind_json(log.response.as_ref());
    bindings.bind_json(log.proxy_server_request.as_ref());
}
