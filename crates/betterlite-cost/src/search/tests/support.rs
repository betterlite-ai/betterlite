use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use rust_decimal::Decimal;

use crate::search::error::SearchCostError;
use crate::search::model_info::SearchModelInfo;
use crate::search::params::SearchOptionalParams;
use crate::search::{SearchPricingSource, search_provider_cost_per_query};

#[derive(Debug, thiserror::Error)]
#[error("no pricing for {0}")]
pub(super) struct MissingPricing(pub(super) String);

pub(super) type Lookup = (String, Option<String>);

pub(super) struct FakeSource {
    records: Mutex<BTreeMap<String, SearchModelInfo>>,
    calls: Mutex<Vec<Lookup>>,
}

impl FakeSource {
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        let records = entries
            .iter()
            .map(|(model, record)| {
                let info = serde_json::from_str(record).expect("the fixture is a model info");
                ((*model).to_owned(), info)
            })
            .collect();
        Self {
            records: Mutex::new(records),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn set(&self, model: &str, info: SearchModelInfo) {
        self.records
            .lock()
            .expect("the records lock is not poisoned")
            .insert(model.to_owned(), info);
    }

    pub(super) fn lookups(&self) -> Vec<Lookup> {
        self.calls
            .lock()
            .expect("the calls lock is not poisoned")
            .clone()
    }
}

impl SearchPricingSource for FakeSource {
    type Error = MissingPricing;

    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> impl Future<Output = Result<SearchModelInfo, MissingPricing>> + Send {
        self.calls
            .lock()
            .expect("the calls lock is not poisoned")
            .push((model.to_owned(), custom_llm_provider.map(str::to_owned)));
        ready(
            self.records
                .lock()
                .expect("the records lock is not poisoned")
                .get(model)
                .cloned()
                .ok_or_else(|| MissingPricing(model.to_owned())),
        )
    }
}

pub(super) fn money(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

pub(super) fn params(json: &str) -> SearchOptionalParams {
    serde_json::from_str(json).expect("the fixture is a set of optional params")
}

pub(super) async fn cost(
    source: &FakeSource,
    model: &str,
    custom_llm_provider: Option<&str>,
    number_of_queries: u64,
    optional_params: &str,
) -> Result<(Decimal, Decimal), SearchCostError<MissingPricing>> {
    search_provider_cost_per_query(
        source,
        model,
        custom_llm_provider,
        number_of_queries,
        &params(optional_params),
    )
    .await
}
