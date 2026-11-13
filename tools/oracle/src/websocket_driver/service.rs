use std::collections::{BTreeMap, btree_map::Entry};

use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{As, PickFirst, Same};

use super::{DriverError, TranscriptObservation};
use crate::websocket_oracle::Frame;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTag {
    NativeResponsesRelay,
    ManagedResponsesTurns,
    RealtimeProbe,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInput {
    pub service: ServiceTag,
    pub input: Arguments,
}

#[derive(Serialize)]
pub(crate) struct ServiceInputWire<'wire> {
    pub service: ServiceTag,
    pub input: ArgumentsWire<'wire>,
}

impl ServiceInput {
    pub(crate) fn to_wire(&self) -> ServiceInputWire<'_> {
        ServiceInputWire {
            service: self.service,
            input: self.input.to_wire(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arguments<F = Frame> {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default, with = "As::<Option<PickFirst<(Vec<(Same, Same)>, Same)>>>")]
    pub query_params: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub openai_beta_realtime: bool,
    #[serde(default = "Vec::new")]
    pub frames: Vec<F>,
}

#[derive(Serialize)]
pub(crate) struct ArgumentsWire<'wire> {
    model: &'wire str,
    api_base: &'wire str,
    api_key: Option<&'wire str>,
    #[serde(with = "As::<Option<PickFirst<(Vec<(Same, Same)>, Same)>>>")]
    query_params: Option<IndexMap<String, String>>,
    openai_beta_realtime: bool,
    frames: Vec<Frame>,
}

impl Arguments {
    pub(crate) fn to_wire(&self) -> ArgumentsWire<'_> {
        ArgumentsWire {
            model: &self.model,
            api_base: &self.api_base,
            api_key: self.api_key.as_ref().map(ExposeSecret::expose_secret),
            query_params: self.query_params.clone(),
            openai_beta_realtime: self.openai_beta_realtime,
            frames: self.frames.clone(),
        }
    }
}

fn default_model() -> String {
    "realtime-a".to_owned()
}
fn default_api_base() -> String {
    "https://api.openai.com/".to_owned()
}
impl<F> Default for Arguments<F> {
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
