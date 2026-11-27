use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_key_budgets::money::Money;
use crate::auth_pricing::cache::ZeroCostCache;
use crate::auth_pricing::catalog::{
    ModelGroupCosts, PricingCatalog, PricingDeployment, PricingError,
};

// the model catalog a test scripts: group costs, deployments, resolved model info and the raw cost map
#[derive(Debug, Default)]
pub struct FakeCatalog {
    pub groups: BTreeMap<String, ModelGroupCosts>,
    pub deployments: BTreeMap<String, Vec<PricingDeployment>>,
    pub infos: BTreeMap<(String, String), JsonObject>,
    pub costs: BTreeMap<String, JsonObject>,
    pub cache: Option<ZeroCostCache>,
    failing: AtomicBool,
    lookups: Mutex<Vec<String>>,
}

impl FakeCatalog {
    #[must_use]
    pub fn with_cache() -> Self {
        Self {
            groups: BTreeMap::new(),
            deployments: BTreeMap::new(),
            infos: BTreeMap::new(),
            costs: BTreeMap::new(),
            cache: Some(ZeroCostCache::default()),
            failing: AtomicBool::new(false),
            lookups: Mutex::new(Vec::new()),
        }
    }

    pub fn fail(&self) {
        self.failing.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn cached(&self, model: &str) -> Option<bool> {
        self.cache.as_ref().and_then(|cache| cache.get(model))
    }

    pub fn clear_cache(&self) {
        if let Some(cache) = &self.cache {
            cache.clear();
        }
    }

    pub fn group(&mut self, model: &str, input: Option<Money>, output: Option<Money>) {
        self.groups.insert(
            model.to_owned(),
            ModelGroupCosts {
                input_cost_per_token: input,
                output_cost_per_token: output,
            },
        );
    }

    pub fn deployed(&mut self, model: &str, params: JsonObject, info: JsonObject) {
        self.deployments
            .entry(model.to_owned())
            .or_default()
            .push(PricingDeployment {
                litellm_params: params,
                model_info: info,
            });
    }

    pub fn raw(&mut self, model_id: &str, entry: JsonObject) {
        self.costs.insert(model_id.to_owned(), entry);
    }

    pub fn resolved(&mut self, model_id: &str, model_name: &str, entry: JsonObject) {
        self.infos
            .insert((model_id.to_owned(), model_name.to_owned()), entry);
    }

    fn recorded(&self, kind: &str, argument: &str) {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{kind}:{argument}"));
    }

    fn failed(&self) -> Result<(), PricingError> {
        if self.failing.load(Ordering::Acquire) {
            return Err(PricingError::Unavailable);
        }
        Ok(())
    }
}

impl PricingCatalog for FakeCatalog {
    fn model_group_info(&self, model: &str) -> Result<Option<ModelGroupCosts>, PricingError> {
        self.recorded("group", model);
        self.failed()?;
        Ok(self.groups.get(model).copied())
    }

    fn model_list(&self, model: &str) -> Result<Vec<PricingDeployment>, PricingError> {
        self.recorded("list", model);
        self.failed()?;
        Ok(self.deployments.get(model).cloned().unwrap_or_default())
    }

    fn deployment_model_info(
        &self,
        model_id: &str,
        model_name: &str,
    ) -> Result<Option<JsonObject>, PricingError> {
        self.recorded("info", &format!("{model_id}|{model_name}"));
        self.failed()?;
        Ok(self
            .infos
            .get(&(model_id.to_owned(), model_name.to_owned()))
            .cloned())
    }

    fn raw_cost_entry(&self, model_id: &str) -> Option<&JsonObject> {
        self.costs.get(model_id)
    }

    fn zero_cost_cache(&self) -> Option<&ZeroCostCache> {
        self.cache.as_ref()
    }
}

#[must_use]
pub fn object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[must_use]
pub fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[must_use]
pub fn number(value: &str) -> Json {
    serde_json::from_str(value).expect("the test number parses")
}

#[must_use]
pub fn money(value: &str) -> Money {
    Money::from_text(value).expect("the test money parses")
}

#[derive(Clone, Default)]
pub struct DebugLogs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for DebugLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub fn captured<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    let logs = DebugLogs::default();
    let value = with_default(registry().with(logs.clone()), run);
    let messages = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, messages)
}
