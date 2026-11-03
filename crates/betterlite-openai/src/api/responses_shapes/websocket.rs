use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::future::Future;

pub trait ResponsesClientWebSocket {
    type Error: std::error::Error + Send + Sync;
    fn send_text<'a>(
        &'a self,
        data: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn receive_text(&self) -> impl Future<Output = Result<String, Self::Error>> + Send + '_;
}
pub trait ResponsesBackendWebSocket {
    type Error: std::error::Error + Send + Sync;
    fn recv(
        &self,
        decode: bool,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, Self::Error>> + Send + '_;
    fn send<'a>(
        &'a self,
        message: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn close(&self) -> impl Future<Output = Result<(), Self::Error>> + Send + '_;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesWebSocketMessage {
    Text(String),
    Bytes(Vec<u8>),
}
pub trait PresidioGuardrailCallback {
    type Error: std::error::Error + Send + Sync;
    type Settings;
    fn presidio_settings(&self, data: &BTreeMap<String, Box<RawValue>>) -> Option<Self::Settings>;
    fn check_pii<'a>(
        &'a self,
        text: &'a str,
        output_parse_pii: bool,
        presidio_config: Option<&'a Self::Settings>,
        request_data: &'a BTreeMap<String, Box<RawValue>>,
    ) -> impl Future<Output = Result<String, Self::Error>> + Send + 'a;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRequestDefaults {
    pub fill_missing: BTreeMap<String, Box<RawValue>>,
    pub overrides: BTreeMap<String, Box<RawValue>>,
}

impl WsRequestDefaults {
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
