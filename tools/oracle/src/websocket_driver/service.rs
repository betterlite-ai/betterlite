use std::collections::{BTreeMap, btree_map::Entry};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{DriverError, TranscriptObservation};
use crate::websocket_oracle::Frame;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTag {
    NativeResponsesRelay,
    ManagedResponsesTurns,
    RealtimeProbe,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInput {
    pub service: ServiceTag,
    pub input: Arguments,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Arguments {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub query_params: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub openai_beta_realtime: bool,
    #[serde(default)]
    pub frames: Vec<Frame>,
}

fn default_model() -> String {
    "realtime-a".to_owned()
}
fn default_api_base() -> String {
    "https://api.openai.com/".to_owned()
}
impl Default for Arguments {
    fn default() -> Self {
        Self {
            model: default_model(),
            api_base: default_api_base(),
            api_key: None,
            query_params: None,
            openai_beta_realtime: false,
            frames: Vec::new(),
        }
    }
}

pub type ServiceAdapter = fn(&Arguments) -> Result<TranscriptObservation, DriverError>;

#[derive(Default)]
pub struct Services {
    adapters: BTreeMap<ServiceTag, ServiceAdapter>,
}

impl Services {
    pub fn register_service(
        &mut self,
        tag: ServiceTag,
        adapter: ServiceAdapter,
    ) -> Result<(), DriverError> {
        match self.adapters.entry(tag) {
            Entry::Vacant(entry) => {
                entry.insert(adapter);
                Ok(())
            }
            Entry::Occupied(_) => Err(DriverError::DuplicateAdapter(tag)),
        }
    }

    pub fn run_service(
        &self,
        tag: ServiceTag,
        input: &Arguments,
    ) -> Result<TranscriptObservation, DriverError> {
        let adapter = self
            .adapters
            .get(&tag)
            .ok_or(DriverError::AdapterMissing(tag))?;
        adapter(input)
    }
}
