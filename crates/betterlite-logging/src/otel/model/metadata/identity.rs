use super::common::{field, object, scalar_text, string, string_including_empty, team_metadata};
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct RequestIdentity {
    pub call_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub team_metadata: Option<BTreeMap<String, OtelModelValue>>,
    pub key_hash: Option<String>,
    pub end_user: Option<String>,
    pub provider_model: Option<String>,
    pub request_route: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl RequestIdentity {
    #[must_use]
    pub fn from_payload(payload: &OtelModelValue) -> Self {
        let metadata = object(field(payload, "metadata"));
        Self {
            call_id: string(field(payload, "litellm_call_id"))
                .or_else(|| string(field(payload, "id"))),
            team_id: string(metadata.and_then(|values| values.get("user_api_key_team_id")))
                .or_else(|| string(metadata.and_then(|values| values.get("team_id")))),
            team_alias: string(metadata.and_then(|values| values.get("user_api_key_team_alias")))
                .or_else(|| string(metadata.and_then(|values| values.get("team_alias")))),
            team_metadata: team_metadata(
                metadata.and_then(|values| values.get("user_api_key_team_metadata")),
            ),
            key_hash: string(metadata.and_then(|values| values.get("user_api_key_hash"))),
            end_user: string(field(payload, "end_user")).or_else(|| {
                string(metadata.and_then(|values| values.get("user_api_key_end_user_id")))
            }),
            provider_model: resolve_provider_model(payload),
            request_route: string_including_empty(
                metadata.and_then(|values| values.get("user_api_key_request_route")),
            ),
            metadata: flatten_metadata(metadata),
        }
    }

    #[must_use]
    pub fn from_user_api_key_auth(
        auth: &OtelModelValue,
        request_metadata: Option<&OtelModelValue>,
    ) -> Self {
        let mut metadata = flatten_metadata(request_metadata.and_then(|value| object(Some(value))));
        for (key, field_name) in [
            ("user_api_key_user_id", "user_id"),
            ("user_api_key_org_id", "org_id"),
            ("user_api_key_alias", "key_alias"),
            ("user_api_key_end_user_id", "end_user_id"),
        ] {
            if let Some(value) = string(field(auth, field_name)) {
                metadata.insert(key.to_owned(), value);
            }
        }
        Self {
            call_id: None,
            team_id: string(field(auth, "team_id")),
            team_alias: string(field(auth, "team_alias")),
            team_metadata: team_metadata(field(auth, "team_metadata")),
            key_hash: string(field(auth, "api_key")),
            end_user: string(field(auth, "end_user_id")),
            provider_model: None,
            request_route: None,
            metadata,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct RequestContext {
    pub request_model: String,
    pub response_model: Option<String>,
    pub model_group: Option<String>,
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub identity: RequestIdentity,
}

impl RequestContext {
    #[must_use]
    pub fn from_standard_logging_payload(payload: &OtelModelValue) -> Self {
        let metadata = object(field(payload, "metadata"));
        let hidden = object(field(payload, "hidden_params"));
        let response = object(field(payload, "response"));
        let model_group = string(field(payload, "model_group"))
            .or_else(|| string(metadata.and_then(|values| values.get("model_group"))));
        let model_id = string(field(payload, "model_id")).or_else(|| {
            string(
                metadata
                    .and_then(|values| values.get("model_info"))
                    .and_then(|value| field(value, "id")),
            )
        });
        let request_model = model_group
            .clone()
            .or_else(|| string(field(payload, "model")))
            .unwrap_or_default();
        Self {
            request_model,
            response_model: string(response.and_then(|values| values.get("model"))),
            model_group,
            model_id,
            api_base: string(field(payload, "api_base"))
                .or_else(|| string(hidden.and_then(|values| values.get("api_base")))),
            identity: RequestIdentity::from_payload(payload),
        }
    }
}

#[must_use]
pub fn flatten_metadata(
    metadata: Option<&BTreeMap<String, OtelModelValue>>,
) -> BTreeMap<String, String> {
    fn collect(
        prefix: &str,
        values: &BTreeMap<String, OtelModelValue>,
        output: &mut BTreeMap<String, String>,
    ) {
        for (key, value) in values {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match object(Some(value)) {
                Some(nested) => collect(&path, nested, output),
                None => output.extend(scalar_text(value).map(|text| (path, text))),
            }
        }
    }
    let mut output = BTreeMap::new();
    if let Some(metadata_fields) = metadata {
        collect("", metadata_fields, &mut output);
    }
    output
}

#[must_use]
pub fn resolve_provider_model(payload: &OtelModelValue) -> Option<String> {
    string(field(payload, "metadata").and_then(|metadata| field(metadata, "deployment")))
        .or_else(|| {
            string(
                field(payload, "hidden_params")
                    .and_then(|hidden| field(hidden, "litellm_model_name")),
            )
        })
        .or_else(|| string(field(payload, "model")))
}
