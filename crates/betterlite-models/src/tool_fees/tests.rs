#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use rust_decimal::Decimal;

use crate::cost_usage::tool_projection::ToolUsage;
use crate::tool_fees::billing::BuiltInToolParams;
use crate::tool_fees::error::LookupFailed;
use crate::tool_fees::model_info::{Rate, ToolModelInfo};
use crate::tool_fees::ports::{ToolPricePolicy, ToolPriceSource};
use crate::tool_fees::prices::ToolPrices;

pub(super) const PRICES: ToolPrices = ToolPrices::PINNED;

pub(super) fn money(units: i64, scale: u32) -> Decimal {
    Decimal::new(units, scale)
}

#[derive(Default)]
pub(super) struct FakeSource {
    pub(super) entries: BTreeMap<(String, Option<String>), ToolModelInfo>,
    pub(super) containers: BTreeMap<String, Result<Rate, LookupFailed>>,
    pub(super) lookups: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeSource {
    pub(super) fn with_entry(
        mut self,
        model: &str,
        provider: Option<&str>,
        info: ToolModelInfo,
    ) -> Self {
        self.entries
            .insert((model.to_owned(), provider.map(str::to_owned)), info);
        self
    }

    pub(super) fn asked(&self) -> Vec<(String, Option<String>)> {
        self.lookups.lock().expect("lookup log").clone()
    }

    pub(super) fn with_container(
        mut self,
        provider: &str,
        rate: Result<Rate, LookupFailed>,
    ) -> Self {
        self.containers.insert(provider.to_owned(), rate);
        self
    }
}

impl ToolPriceSource for FakeSource {
    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<ToolModelInfo, LookupFailed> {
        let key = (model.to_owned(), custom_llm_provider.map(str::to_owned));
        if let Ok(mut log) = self.lookups.lock() {
            log.push(key.clone());
        }
        self.entries.get(&key).cloned().ok_or(LookupFailed)
    }

    fn container_session_rate(&self, provider: &str) -> Result<Rate, LookupFailed> {
        if let Ok(mut log) = self.lookups.lock() {
            log.push((format!("{provider}/container"), None));
        }
        self.containers
            .get(provider)
            .copied()
            .unwrap_or(Err(LookupFailed))
    }
}

#[derive(Default)]
pub(super) struct FakePolicy {
    pub(super) web: Option<Decimal>,
    pub(super) maps: Option<Decimal>,
    pub(super) calls: Mutex<Vec<String>>,
}

impl FakePolicy {
    pub(super) fn seen(&self) -> Vec<String> {
        self.calls.lock().expect("call log").clone()
    }
}

impl ToolPricePolicy for FakePolicy {
    fn web_search_cost(
        &self,
        provider: &str,
        usage: &ToolUsage,
        _model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        let requests = usage
            .server_tool_use
            .and_then(|server| server.web_search_requests);
        self.calls
            .lock()
            .expect("call log")
            .push(format!("web:{provider}:{requests:?}"));
        self.web
    }

    fn google_maps_cost(
        &self,
        provider: &str,
        requests: i64,
        _model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        self.calls
            .lock()
            .expect("call log")
            .push(format!("maps:{provider}:{requests}"));
        self.maps
    }
}

pub(super) fn info_with<Edit>(edit: Edit) -> ToolModelInfo
where
    Edit: FnOnce(&mut ToolModelInfo),
{
    let mut info = ToolModelInfo::default();
    edit(&mut info);
    info
}

pub(super) fn params_with<Edit>(edit: Edit) -> BuiltInToolParams
where
    Edit: FnOnce(&mut BuiltInToolParams),
{
    let mut params = BuiltInToolParams::default();
    edit(&mut params);
    params
}

pub(super) fn policy(web: Option<Decimal>, maps: Option<Decimal>) -> FakePolicy {
    FakePolicy {
        web,
        maps,
        calls: Mutex::new(Vec::new()),
    }
}
