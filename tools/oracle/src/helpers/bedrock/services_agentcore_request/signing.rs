use std::future::ready;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_bedrock::agentcore_request::signing::{SignRequest, sign_request};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::target::RegionInputs;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::{SignRun, error_kind, header_map, optional_headers};

fn credentials_of(run: &SignRun) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(run.credentials.access_key_id.clone()),
        secret_access_key: SecretString::from(run.credentials.secret_access_key.clone()),
        session_token: run
            .credentials
            .session_token
            .as_ref()
            .map(|token| SecretString::from(token.clone())),
    }
}

pub(super) fn sign_result(run: &SignRun) -> Value {
    let credentials = credentials_of(run);
    let api_key = run
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.clone()));
    let configured = run
        .configured_bearer
        .as_ref()
        .map(|token| SecretString::from(token.clone()));
    let headers = optional_headers(&run.headers);
    let now = UNIX_EPOCH
        .checked_add(Duration::from_secs(run.now))
        .unwrap_or(UNIX_EPOCH);
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return json!({"ok": false, "error": error.to_string()}),
    };
    match runtime.block_on(sign_request(
        SignRequest {
            headers: &headers,
            optional_params: &run.optional_params,
            request_data: &run.request_data,
            api_base: &run.api_base,
            model: Some(&run.model),
            api_key: api_key.as_ref(),
            configured_bearer: configured.as_ref(),
            region_inputs: RegionInputs::default(),
            now,
        },
        move |_settings, _region| ready(Ok(credentials)),
    )) {
        Ok(signed) => signed_result(&signed),
        Err(error) => json!({"ok": false, "error": error_kind(&error)}),
    }
}

fn signed_result(signed: &AwsSignedRequest) -> Value {
    String::from_utf8(signed.body.clone()).map_or_else(
        |_| json!({"ok": false, "error": "non_utf8_body"}),
        |text| {
            json!({
                "ok": true,
                "method": signed.method,
                "url": signed.url,
                "headers": header_map(&signed.headers),
                "body": text,
            })
        },
    )
}
