use std::time::Duration;

use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};

use super::slots::LoadedHook;

pub const HOOK_DEADLINE: Duration = Duration::from_secs(30);
pub(crate) const LOAD_DEADLINE: Duration = Duration::from_secs(60);

pub type HookResult = Result<JsonValue, CallError>;

impl LoadedHook {
    pub async fn call(&self, args: Vec<JsonValue>, kwargs: JsonObject) -> HookResult {
        // the default hook deadline, unless the caller passes its own
        self.call_within(HOOK_DEADLINE, args, kwargs).await
    }

    pub async fn call_within(
        &self,
        deadline: Duration,
        args: Vec<JsonValue>,
        kwargs: JsonObject,
    ) -> HookResult {
        let handle = self.object.loaded.handle(&self.host)?;
        handle
            .call(&self.host, &self.tracker, deadline, args, kwargs)
            .await
    }
}

mod tests;
