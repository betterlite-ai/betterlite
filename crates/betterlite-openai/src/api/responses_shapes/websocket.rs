use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

pub trait ResponsesClientWebSocket {
    type Error: Error + Send + Sync;
    fn send_text<'call>(
        &'call self,
        data: &'call str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'call;
    fn receive_text(&self) -> impl Future<Output = Result<String, Self::Error>> + Send + '_;
}
pub trait ResponsesBackendWebSocket {
    type Error: Error + Send + Sync;
    fn recv(
        &self,
        decode: bool,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, Self::Error>> + Send + '_;
    fn send<'call>(
        &'call self,
        message: &'call str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'call;
    fn close(&self) -> impl Future<Output = Result<(), Self::Error>> + Send + '_;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesWebSocketMessage {
    Text(String),
    Bytes(Vec<u8>),
}
pub trait PresidioGuardrailCallback {
    type Error: Error + Send + Sync;
    type Settings;
    fn presidio_settings(&self, data: &BTreeMap<String, Box<RawValue>>) -> Option<Self::Settings>;
    fn check_pii<'call>(
        &'call self,
        text: &'call str,
        output_parse_pii: bool,
        presidio_config: Option<&'call Self::Settings>,
        request_data: &'call BTreeMap<String, Box<RawValue>>,
    ) -> impl Future<Output = Result<String, Self::Error>> + Send + 'call;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRequestDefaults {
    pub fill_missing: BTreeMap<String, Box<RawValue>>,
    pub overrides: BTreeMap<String, Box<RawValue>>,
}

impl WsRequestDefaults {
    #[must_use]
    pub fn merged_into(
        &self,
        request: &BTreeMap<String, Box<RawValue>>,
    ) -> BTreeMap<String, Box<RawValue>> {
        let mut merged = self.fill_missing.clone();
        merged.extend(request.clone());
        merged.extend(self.overrides.clone());
        merged
    }
}

mod tests;
