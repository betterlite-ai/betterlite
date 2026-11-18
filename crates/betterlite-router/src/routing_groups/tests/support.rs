use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use tracing::subscriber::{DefaultGuard, set_default};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::services::RouteServices;
use crate::router_predicates::JsonMap;
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::routing_groups::group::RoutingGroup;

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn info(entries: &[(&str, Json)]) -> JsonMap {
    let mut map = JsonMap::new();
    for (key, value) in entries {
        map.insert((*key).to_owned(), value.clone());
    }
    map
}

pub(super) fn object(entries: &[(&str, Json)]) -> Json {
    Json::Object(info(entries))
}

pub(super) fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn group(name: &str, models: &[&str], strategy: &str) -> RoutingGroup {
    RoutingGroup::new(name.to_owned(), texts(models), strategy, None, None).unwrap()
}

pub(super) fn priority_group(name: &str, declared: &[(&str, u64)]) -> RoutingGroup {
    let models: Vec<String> = declared
        .iter()
        .map(|(model, _)| (*model).to_owned())
        .collect();
    let priorities: BTreeMap<String, u64> = declared
        .iter()
        .map(|(model, priority)| ((*model).to_owned(), *priority))
        .collect();
    RoutingGroup::new(name.to_owned(), models, "priority", None, Some(priorities)).unwrap()
}

pub(super) fn raw(name: &str, models: &[&str], strategy: &str) -> JsonMap {
    info(&[
        ("group_name", text(name)),
        (
            "models",
            Json::List(models.iter().map(|model| text(model)).collect()),
        ),
        ("routing_strategy", text(strategy)),
    ])
}

pub(super) fn with_priorities(mut group: JsonMap, priorities: &[(&str, Json)]) -> JsonMap {
    group.insert(
        "model_priorities".to_owned(),
        Json::Object(info(priorities)),
    );
    group
}

pub(super) fn with_args(mut group: JsonMap, args: &[(&str, Json)]) -> JsonMap {
    group.insert("routing_strategy_args".to_owned(), Json::Object(info(args)));
    group
}

pub(super) fn deployment(model_name: &str, id: &str) -> DeploymentSpec {
    let mut litellm = JsonObject::new();
    litellm.insert("model".to_owned(), text(&format!("openai/{id}")));
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), text(id));
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

// A provider registry that resolves the provider when `resolves` is set, else keeps every param.
pub(super) struct Registry {
    pub(super) resolves: bool,
    pub(super) params: &'static [&'static str],
}

impl ParamSupport for Registry {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        self.resolves.then(|| ResolvedProvider {
            model: model.to_owned(),
            custom_llm_provider: "openai".to_owned(),
        })
    }

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        self.resolves
            .then(|| self.params.iter().map(|name| (*name).to_owned()).collect())
    }
}

pub(super) const SUPPORTS_SEED: Registry = Registry {
    resolves: true,
    params: &["seed"],
};

pub(super) const SUPPORTS_TEMPERATURE: Registry = Registry {
    resolves: true,
    params: &["temperature"],
};

pub(super) const UNRESOLVED: Registry = Registry {
    resolves: false,
    params: &[],
};

// The router's services with one callable group, which answers only for its own name.
pub(super) struct GroupServices {
    pub(super) name: String,
    pub(super) group: RoutingGroup,
}

impl RouteServices for GroupServices {
    fn routing_group(&self, model: &str) -> Option<&RoutingGroup> {
        (model == self.name).then_some(&self.group)
    }
}

// The log lines a call writes, through a subscriber only this thread sees.
#[derive(Clone)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
