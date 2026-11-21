use std::collections::BTreeMap;
use std::ops::Div as _;

use betterlite_auth::auth_request_policy::invalid_key::{
    ErrorClass, ErrorFacts, MarkOutcome, Marker, is_invalid_virtual_key_error,
    mark_invalid_virtual_key_error,
};
use betterlite_auth::auth_request_policy::ip::{
    RequestProvenance, check_valid_ip, request_ip_address,
};
use betterlite_auth::auth_request_policy::policy::{PreDbRequest, pre_db_read_auth_checks};
use betterlite_auth::request_body_safety::{
    FormMetadataExtractor, ModelGrantLookup, ModelGrants, RequestBodySafety,
};
use betterlite_config::sections::auth_request_policy::Settings;
use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_config::sections::request_body_safety::Settings as BodySafetySettings;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};

use super::{Call, ClassifyCase, ErrorKind, IpCase, MarkCase, MarkerName, PreDbCase};

const AUTH_ERROR: &str = "auth_error";
const PARAM_NONE: &str = "None";
const MEBIBYTE: f64 = 1_048_576.0;

pub(super) fn answer(call: &Call) -> Value {
    match call {
        Call::BytesToMb { values } => {
            json!(
                values
                    .iter()
                    .map(|value| value.to_f64().map(|megabytes| megabytes.div(MEBIBYTE)))
                    .collect::<Vec<_>>()
            )
        }
        Call::Ip { requests } => json!(requests.iter().map(ip_answer).collect::<Vec<_>>()),
        Call::Classify { rows } => json!(rows.iter().map(classify).collect::<Vec<_>>()),
        Call::Mark { rows } => json!(rows.iter().map(mark).collect::<Vec<_>>()),
        Call::PreDb { requests } => json!(requests.iter().map(pre_db).collect::<Vec<_>>()),
    }
}

fn ip_answer(request: &IpCase) -> Value {
    let provenance = RequestProvenance {
        forwarded_for: request.forwarded_for.as_deref(),
        client_host: request.client_host.as_deref(),
    };
    let ip = request_ip_address(&provenance, request.use_x_forwarded_for);
    let verdict = check_valid_ip(
        request.allowed_ips.as_deref(),
        &provenance,
        request.use_x_forwarded_for,
    );
    json!({
        "ip": ip,
        "allowed": verdict.allowed,
        "passed_in_ip": verdict.passed_in_ip,
    })
}

fn classify(row: &ClassifyCase) -> bool {
    match row.kind {
        ErrorKind::Proxy => classify_proxy(row),
        ErrorKind::Http => is_invalid_virtual_key_error(&facts(row, ErrorClass::Http)),
        ErrorKind::Other => is_invalid_virtual_key_error(&facts(row, ErrorClass::Other)),
    }
}

fn facts<'row>(row: &'row ClassifyCase, class: ErrorClass) -> ErrorFacts<'row> {
    ErrorFacts {
        class,
        code: row.code.as_deref(),
        status_code: Some(row.status_code),
        marker: marker(row.marker),
    }
}

fn classify_proxy(row: &ClassifyCase) -> bool {
    let exception = row_exception(&row.message, row.code.clone(), BTreeMap::new(), None);
    is_invalid_virtual_key_error(&ErrorFacts {
        marker: marker(row.marker),
        ..ErrorFacts::proxy(&exception)
    })
}

fn row_exception(
    message: &str,
    code: Option<String>,
    headers: BTreeMap<String, String>,
    openai_code: Option<String>,
) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: message.to_owned(),
        error_type: AUTH_ERROR.to_owned(),
        param: Some(PARAM_NONE.to_owned()),
        code: code.map_or(ExceptionCode::Absent, ExceptionCode::Text),
        headers,
        openai_code,
        provider_specific_fields: BTreeMap::new(),
    })
}

const fn marker(name: MarkerName) -> Marker {
    match name {
        MarkerName::Absent => Marker::Absent,
        MarkerName::NotTrue => Marker::NotTrue,
        MarkerName::ExactTrue => Marker::ExactTrue,
    }
}

fn mark(row: &MarkCase) -> Value {
    let exception = row_exception(
        &row.message,
        Some(row.code.clone()),
        row.headers.clone(),
        row.openai_code.clone(),
    );
    let outcome = mark_invalid_virtual_key_error(&exception, row.is_invalid_virtual_key);
    let marked = match &outcome {
        MarkOutcome::Same => &exception,
        MarkOutcome::Marked(copy) => copy.as_ref(),
    };
    let classified = is_invalid_virtual_key_error(&ErrorFacts {
        marker: outcome.marker(),
        ..ErrorFacts::proxy(marked)
    });
    json!({
        "same_object": matches!(&outcome, MarkOutcome::Same),
        "classified": classified,
        "type": marked.error_type,
        "param": marked.param,
        "code": marked.code,
        "openai_code": marked.openai_code,
        "headers": marked.headers,
    })
}

fn pre_db(call: &PreDbCase) -> Value {
    let settings = Settings {
        allowed_ips: call.allowed_ips.clone(),
        use_x_forwarded_for: call.use_x_forwarded_for,
        max_request_size_mb: call.max_request_size_mb,
        max_response_size_mb: None,
        allowed_routes: call.allowed_routes.clone(),
    };
    let scan_defaults = BodySafetySettings::default();
    let grants = NoGrants(BTreeMap::new());
    let metadata = NoMetadata;
    let safety = RequestBodySafety {
        body: &call.body,
        model: model_name(&call.body),
        allow_client_side_credentials: scan_defaults.allow_client_side_credentials,
        allowed_hosts: &scan_defaults.allowed_hosts,
        grants: &grants,
        metadata: &metadata,
        max_fallbacks: scan_defaults.max_fallbacks,
    };
    let request = PreDbRequest {
        content_length: call.content_length.as_deref(),
        // the Python request carries no body bytes, so a missing content-length measures zero too
        body: &[],
        route: &call.route,
        provenance: RequestProvenance {
            forwarded_for: call.forwarded_for.as_deref(),
            client_host: call.client_host.as_deref(),
        },
        body_safety: Some(safety),
    };
    match pre_db_read_auth_checks(request, &settings) {
        Ok(()) => accepted(),
        Err(error) => rejected(error.into_proxy_exception()),
    }
}

fn model_name(body: &JsonObject) -> &str {
    match body.get("model") {
        Some(Json::Text(model)) => model.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    }
}

fn accepted() -> Value {
    json!({
        "ok": true,
        "status": null,
        "type": null,
        "param": null,
        "code": null,
        "message": null,
    })
}

fn rejected(exception: ProxyException) -> Value {
    json!({
        "ok": false,
        "status": exception.code.parse::<i64>().ok(),
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
        "message": exception.message,
    })
}

// llm_router is None in the injected Python globals, so no model group carries a grant
#[derive(Debug)]
struct NoGrants(BTreeMap<String, Option<Vec<ClientsideParamGrant>>>);

impl ModelGrantLookup for NoGrants {
    fn grants(&self, model_group: &str) -> ModelGrants<'_> {
        match self.0.get(model_group) {
            None => ModelGrants::Absent,
            Some(None) => ModelGrants::Unset,
            Some(Some(items)) => ModelGrants::Granted(items),
        }
    }
}

#[derive(Debug)]
struct NoMetadata;

impl FormMetadataExtractor for NoMetadata {
    fn top_level_names(&self, form_data: &JsonObject, prefix: &str) -> Vec<String> {
        form_data
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }
}
