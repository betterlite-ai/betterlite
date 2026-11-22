#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_core::api::response_id_codecs::encrypted::{
    build_encrypted_item_id, wrap_encrypted_content,
};
use betterlite_core::api::response_id_traversal::record::Record;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::responses_affinity::config::AffinitySettings;
use crate::responses_affinity::error::{AffinityError, error_type_name};
use crate::responses_affinity::filter::AffinityCheck;
use crate::responses_affinity::history::{FilterRequest, History, MetadataSlot};
use crate::responses_affinity::identity::deployment_id;
use crate::routing_groups::index::RoutingGroupIndex;

mod boundary;
mod enablement;
mod fallback;
mod markers;
mod peers;
mod pin;
mod strip;

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn prompt_text(value: &str) -> PromptJson {
    PromptJson::Text(value.to_owned())
}

pub(super) fn prompt_object(entries: &[(&str, PromptJson)]) -> PromptObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn params(base: Option<&str>, key: Option<&str>) -> JsonObject {
    let mut params = object(&[("model", text("openai/a"))]);
    if let Some(deployment_base) = base {
        params.insert("api_base".to_owned(), text(deployment_base));
    }
    if let Some(deployment_key) = key {
        params.insert("api_key".to_owned(), text(deployment_key));
    }
    params
}

pub(super) fn params_with_credential(credential: &str) -> JsonObject {
    let mut params = params(Some("deployment-base"), Some("deployment-key"));
    params.insert("litellm_credential_name".to_owned(), text(credential));
    params
}

pub(super) fn info(id: &str) -> JsonObject {
    object(&[("id", text(id))])
}

pub(super) fn spec(id: &str, params: JsonObject) -> DeploymentSpec {
    DeploymentSpec::new(
        "gpt-group".to_owned(),
        params,
        info(id),
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(super) fn spec_with_model_info(model_info: JsonObject) -> DeploymentSpec {
    DeploymentSpec {
        model_name: "gpt-group".to_owned(),
        litellm_params: params(Some("u"), Some("k")),
        model_info,
        extra: JsonObject::new(),
    }
}

pub(super) fn credentials(entries: &[(&str, JsonObject)]) -> CredentialSnapshot {
    CredentialSnapshot::new(
        entries
            .iter()
            .map(|(name, values)| CredentialEntry {
                credential_name: (*name).to_owned(),
                credential_values: values.clone(),
            })
            .collect(),
    )
}

pub(super) fn item_marker(model_id: &str) -> String {
    build_encrypted_item_id(model_id, "r1")
}

pub(super) fn wrapped_marker(model_id: &str) -> String {
    wrap_encrypted_content("cipher", model_id)
}

pub(super) fn reasoning_item(model_id: &str) -> Json {
    Json::Object(object(&[
        ("type", text("reasoning")),
        ("id", text(&item_marker(model_id))),
        ("encrypted_content", text(&wrapped_marker(model_id))),
    ]))
}

pub(super) fn summary_list() -> Json {
    Json::List(vec![Json::Object(object(&[
        ("type", text("summary_text")),
        ("text", text("readable")),
    ]))])
}

pub(super) fn titled_item(model_id: &str) -> Json {
    Json::Object(object(&[
        ("type", text("reasoning")),
        ("id", text(&item_marker(model_id))),
        ("encrypted_content", text("cipher")),
        ("summary", summary_list()),
    ]))
}

pub(super) fn kept_titled_item() -> Json {
    Json::Object(object(&[
        ("type", text("reasoning")),
        ("summary", summary_list()),
    ]))
}

pub(super) fn message_item() -> Json {
    Json::Object(object(&[
        ("type", text("message")),
        ("role", text("user")),
        ("content", text("hello")),
    ]))
}

pub(super) fn message(parts: Vec<ContentPart>) -> Message {
    Message {
        role: "assistant".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: PromptObject::new(),
    }
}

pub(super) fn thinking_part(model_id: Option<&str>) -> ContentPart {
    let signature = model_id.map_or_else(
        || "anthropic-signature".to_owned(),
        |origin| format!("litellm_encrypted_reasoning:{}", wrapped_marker(origin)),
    );
    ContentPart {
        kind: "thinking".to_owned(),
        fields: prompt_object(&[
            ("thinking", prompt_text("t")),
            ("signature", prompt_text(&signature)),
        ]),
    }
}

pub(super) fn redacted_part(model_id: &str) -> ContentPart {
    let data = format!("litellm_encrypted_reasoning:{}", wrapped_marker(model_id));
    ContentPart {
        kind: "redacted_thinking".to_owned(),
        fields: prompt_object(&[("data", prompt_text(&data))]),
    }
}

pub(super) fn text_part(value: &str) -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: prompt_object(&[("text", prompt_text(value))]),
    }
}

pub(super) fn settings(global: bool, groups: &[(&str, &[&str])]) -> AffinitySettings {
    let config = groups
        .iter()
        .map(|(name, checks)| {
            (
                (*name).to_owned(),
                checks.iter().map(|check| (*check).to_owned()).collect(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    AffinitySettings {
        enable_global_affinity: global,
        model_group_affinity_config: config,
    }
}

pub(super) fn ids(deployments: &[DeploymentSpec]) -> Vec<Option<String>> {
    deployments.iter().map(deployment_id).collect()
}

pub(super) fn check<'check>(
    settings: AffinitySettings,
    credentials: &'check CredentialSnapshot,
) -> AffinityCheck<'check, 'static> {
    AffinityCheck::new(settings, None, credentials)
}

pub(super) fn catalog_check<'check, 'catalog>(
    settings: AffinitySettings,
    lookup: &'check CatalogLookup<'catalog>,
    credentials: &'check CredentialSnapshot,
) -> AffinityCheck<'check, 'catalog> {
    AffinityCheck::new(settings, Some(lookup), credentials)
}

pub(super) const fn no_provider(_model: &str) -> Option<String> {
    None
}

pub(super) struct Fixture {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
}

impl Fixture {
    pub(super) fn new(rows: Vec<DeploymentSpec>) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for row in rows {
            catalog.index_patterns(&row).unwrap();
            catalog.push(row);
        }
        Self {
            catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
        }
    }

    pub(super) fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &no_provider,
        }
    }
}

pub(super) struct Request {
    pub(super) input: Option<Json>,
    pub(super) messages: Option<Vec<Message>>,
    pub(super) request_messages: Option<Vec<Message>>,
    pub(super) metadata: Option<Json>,
    pub(super) pinned: bool,
}

impl Request {
    pub(super) fn new() -> Self {
        Self {
            input: None,
            messages: None,
            request_messages: None,
            metadata: None,
            pinned: false,
        }
    }

    pub(super) fn with_items(items: Vec<Json>) -> Self {
        let mut request = Self::new();
        request.input = Some(Json::List(items));
        request
    }

    pub(super) fn filter(
        &mut self,
        check: &AffinityCheck<'_, '_>,
        model: &str,
        deployments: &[DeploymentSpec],
    ) -> Result<Vec<DeploymentSpec>, AffinityError> {
        let mut input = self.input.take().map(Record::from_json);
        let metadata = match self.metadata.as_mut() {
            None => MetadataSlot::Absent,
            Some(Json::Object(fields)) => MetadataSlot::Object(fields),
            Some(value) => MetadataSlot::NotAnObject {
                python_type: error_type_name(value),
            },
        };
        let history = History {
            input: input.as_mut(),
            messages: self.messages.as_mut(),
            request_messages: self.request_messages.as_mut(),
            metadata,
            pinned: &mut self.pinned,
        };
        let outcome = check.filter(FilterRequest {
            model,
            deployments,
            history,
        });
        self.input = input.map(Record::into_json);
        outcome
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::DEBUG && level != Level::WARN {
            return;
        }
        let mut message = MessageField::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn captured<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, run);
    let messages = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, messages)
}
