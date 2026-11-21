#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use rust_decimal::Decimal;

use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::savings_pricing::context::{
    ModelIdentity, SavingsPricingContext, public_price, resolve_model,
};
use crate::tiered_pricing::Scalar;

#[derive(Default)]
pub(crate) struct TestContext {
    identities: Vec<(String, Option<String>, ModelIdentity)>,
    public: Vec<(String, String, PriceRecord)>,
    deployments: BTreeMap<String, PriceRecord>,
}

impl TestContext {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_identity(
        mut self,
        model: &str,
        provider: Option<&str>,
        canonical: (&str, &str),
    ) -> Self {
        self.identities.push((
            model.to_owned(),
            provider.map(str::to_owned),
            ModelIdentity {
                model: canonical.0.to_owned(),
                provider: canonical.1.to_owned(),
            },
        ));
        self
    }

    pub(crate) fn with_public(mut self, model: &str, provider: &str, record: PriceRecord) -> Self {
        self.public
            .push((model.to_owned(), provider.to_owned(), record));
        self
    }

    pub(crate) fn with_deployment(mut self, id: &str, record: PriceRecord) -> Self {
        self.deployments.insert(id.to_owned(), record);
        self
    }

    fn public_record(&self, model: &str, provider: Option<&str>) -> Option<&PriceRecord> {
        let provider_name = provider.unwrap_or_default();
        self.public
            .iter()
            .find(|entry| entry.0 == model && entry.1 == provider_name)
            .map(|entry| &entry.2)
    }
}

impl SavingsPricingContext for TestContext {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        self.identities
            .iter()
            .find(|entry| entry.0 == model && entry.1.as_deref() == custom_llm_provider)
            .map(|entry| entry.2.clone())
    }

    fn deployment_price(
        &self,
        deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        let record = deployment_id
            .and_then(|id| self.deployments.get(id))
            .cloned();
        ready(record.map(|stored| priced(&stored)))
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self
    }
}

impl PriceLookup for TestContext {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.public_record(model, custom_llm_provider).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.public
            .iter()
            .find(|entry| entry.0 == key)
            .map(|entry| entry.2.clone())
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.public_record(model, custom_llm_provider).map(priced)
    }
}

pub(crate) fn priced(record: &PriceRecord) -> PricedModel {
    PricedModel {
        record: record.clone(),
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    }
}

pub(crate) fn record(json: &str) -> PriceRecord {
    serde_json::from_str(json).expect("the fixture is a price record")
}

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

pub(crate) fn number(record: &PriceRecord, key: &str) -> Option<Decimal> {
    match record.get(key) {
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(
            Scalar::Null | Scalar::Bool(_) | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_),
        )
        | None => None,
    }
}

#[test]
fn resolve_model_requires_a_nonempty_name() {
    let context = TestContext::new().with_identity("claude", None, ("claude", "anthropic"));
    assert_eq!(resolve_model(&context, None, None), None);
    assert_eq!(resolve_model(&context, Some(""), None), None);
    assert_eq!(
        resolve_model(&context, Some("claude"), None),
        Some(ModelIdentity {
            model: String::from("claude"),
            provider: String::from("anthropic"),
        })
    );
}

#[test]
fn resolve_model_rejects_an_empty_name_even_when_the_context_knows_it() {
    let context = TestContext::new().with_identity("", None, ("", "anthropic"));
    assert_eq!(resolve_model(&context, Some(""), None), None);
}

#[test]
fn resolve_model_drops_an_empty_custom_provider() {
    let context = TestContext::new().with_identity("claude", None, ("claude", "anthropic"));
    assert!(resolve_model(&context, Some("claude"), Some("")).is_some());
}

#[test]
fn resolve_model_reports_an_unmapped_name() {
    let context = TestContext::new();
    assert_eq!(resolve_model(&context, Some("unknown"), None), None);
}

#[test]
fn public_price_returns_the_flat_record() {
    let context = TestContext::new().with_public(
        "claude",
        "anthropic",
        record(r#"{"input_cost_per_token": 0.5}"#),
    );
    let identity = ModelIdentity {
        model: String::from("claude"),
        provider: String::from("anthropic"),
    };
    let priced = public_price(&context, &identity).expect("the public entry is registered");
    assert_eq!(
        number(&priced.record, "input_cost_per_token"),
        Some(decimal("0.5"))
    );
}

#[test]
fn public_price_reports_a_missing_entry() {
    let context = TestContext::new();
    let identity = ModelIdentity {
        model: String::from("claude"),
        provider: String::from("anthropic"),
    };
    assert!(public_price(&context, &identity).is_none());
}
