mod adapter;
mod payload;
mod tests;
mod transport;

use std::sync::{Arc, Mutex};

use uuid::Uuid;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::galileo::delivery::config::GalileoConfig;
use crate::galileo::delivery::transport::TraceIds;
use crate::galileo::delivery::{GalileoDelivery, State};
use adapter::GalileoAdapter;
use transport::PooledTransport;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    Ok(Arc::new(GalileoAdapter {
        delivery: Arc::new(GalileoDelivery {
            config: GalileoConfig::from_environment(inputs.env.as_ref()),
            state: Mutex::new(State::default()),
        }),
        transport: Arc::new(PooledTransport {
            client: inputs.http.clone(),
        }),
        ids: Arc::new(UuidTraceIds),
        tracker: inputs.tracker.clone(),
        shutdown: inputs.shutdown.clone(),
    }))
}

struct UuidTraceIds;

impl TraceIds for UuidTraceIds {
    fn next_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}
