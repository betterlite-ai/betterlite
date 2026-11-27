use std::sync::Arc;

use thiserror::Error;

use crate::router_call_contract::GenericCallExecutor;
use crate::router_call_contract::error::GenericError;

#[derive(Debug, Error)]
#[error("the call type {0} has two executors")]
pub struct DuplicateCallType(pub String);

pub struct GenericExecutors {
    executors: Vec<(String, Arc<dyn GenericCallExecutor>)>,
}

impl GenericExecutors {
    pub fn new(
        executors: Vec<(String, Arc<dyn GenericCallExecutor>)>,
    ) -> Result<Self, DuplicateCallType> {
        for (index, (call_type, _)) in executors.iter().enumerate() {
            if executors
                .iter()
                .take(index)
                .any(|(earlier, _)| earlier == call_type)
            {
                return Err(DuplicateCallType(call_type.clone()));
            }
        }
        Ok(Self { executors })
    }

    pub fn executor(&self, call_type: &str) -> Result<Arc<dyn GenericCallExecutor>, GenericError> {
        self.executors
            .iter()
            .find(|(name, _)| name.as_str() == call_type)
            .map(|(_, executor)| Arc::clone(executor))
            .ok_or_else(|| GenericError::UnknownCallType(call_type.to_owned()))
    }
}
