pub mod auth;
pub mod buffer;
pub mod config;
pub mod diagnostics;
pub mod error;
pub mod flush;
pub mod headers;
pub mod health;
pub mod transport;

mod tests;

use std::sync::Mutex;

use betterlite_models::decisions::Json;
use config::GalileoConfig;
use headers::Header;
use indexmap::IndexMap;

pub type Record = IndexMap<String, Json>;

#[derive(Debug, Default)]
pub struct State {
    pub records: Vec<Record>,
    pub headers: Option<Vec<Header>>,
}

#[derive(Debug)]
pub struct GalileoDelivery {
    pub config: GalileoConfig,
    pub state: Mutex<State>,
}
