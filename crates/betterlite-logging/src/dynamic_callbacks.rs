pub mod error;
pub mod filters;
pub mod params;
pub mod resolve;

use std::fmt;
use std::sync::Arc;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::logging_privacy::callbacks::TrustedCallbacks;
use crate::logging_state::types::DynamicCallbacks;

use error::DynamicCallbackError;

mod tests;

#[derive(Clone)]
pub enum DynamicCallback {
    Name(String),
    Logger(Arc<dyn CallbackLogger>),
}

impl fmt::Debug for DynamicCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.debug_tuple("Name").field(name).finish(),
            Self::Logger(logger) => f
                .debug_tuple("Logger")
                .field(&logger.callback_name())
                .finish(),
        }
    }
}

impl PartialEq for DynamicCallback {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Name(left), Self::Name(right)) => left == right,
            (Self::Logger(left), Self::Logger(right)) => Arc::ptr_eq(left, right),
            (Self::Name(_), Self::Logger(_)) | (Self::Logger(_), Self::Name(_)) => false,
        }
    }
}

impl Eq for DynamicCallback {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicCallbackKind {
    Input,
    Success,
    AsyncSuccess,
    Failure,
    AsyncFailure,
}

impl DynamicCallbackKind {
    // LiteLLM processes success before async success, so an entry it mirrors there is never resolved twice
    pub const ALL: [Self; 5] = [
        Self::Input,
        Self::Failure,
        Self::AsyncFailure,
        Self::Success,
        Self::AsyncSuccess,
    ];

    const fn mirror(self) -> Option<Self> {
        match self {
            Self::Success => Some(Self::AsyncSuccess),
            Self::Failure => Some(Self::AsyncFailure),
            Self::Input | Self::AsyncSuccess | Self::AsyncFailure => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DynamicCallbackLists {
    pub input: Option<Vec<DynamicCallback>>,
    pub success: Option<Vec<DynamicCallback>>,
    pub async_success: Option<Vec<DynamicCallback>>,
    pub failure: Option<Vec<DynamicCallback>>,
    pub async_failure: Option<Vec<DynamicCallback>>,
}

impl From<&DynamicCallbacks> for DynamicCallbackLists {
    fn from(dynamic: &DynamicCallbacks) -> Self {
        Self {
            input: name_entries(dynamic.input.as_deref()),
            success: name_entries(dynamic.success.as_deref()),
            async_success: name_entries(dynamic.async_success.as_deref()),
            failure: name_entries(dynamic.failure.as_deref()),
            async_failure: name_entries(dynamic.async_failure.as_deref()),
        }
    }
}

fn name_entries(names: Option<&[String]>) -> Option<Vec<DynamicCallback>> {
    let values = names?;
    Some(
        values
            .iter()
            .map(|name| DynamicCallback::Name(name.clone()))
            .collect(),
    )
}

#[must_use]
pub fn with_dynamic_callback(
    existing: Option<&[DynamicCallback]>,
    callback: &DynamicCallback,
) -> Vec<DynamicCallback> {
    let mut updated = existing.unwrap_or_default().to_vec();
    if !updated.contains(callback) {
        updated.push(callback.clone());
    }
    updated
}

impl DynamicCallbackLists {
    pub fn add_dynamic_callback(&mut self, callback: &DynamicCallback) {
        self.input = Some(with_dynamic_callback(self.input.as_deref(), callback));
        self.success = Some(with_dynamic_callback(self.success.as_deref(), callback));
        self.async_success = Some(with_dynamic_callback(
            self.async_success.as_deref(),
            callback,
        ));
        self.failure = Some(with_dynamic_callback(self.failure.as_deref(), callback));
        self.async_failure = Some(with_dynamic_callback(
            self.async_failure.as_deref(),
            callback,
        ));
    }

    pub fn process_dynamic_callbacks(
        &mut self,
        resolver: &dyn LoggerResolver,
        trusted: &TrustedCallbacks,
    ) -> Result<(), DynamicCallbackError> {
        for kind in DynamicCallbackKind::ALL {
            self.process_dynamic_callback_list(kind, resolver, trusted)?;
        }
        Ok(())
    }

    fn process_dynamic_callback_list(
        &mut self,
        kind: DynamicCallbackKind,
        resolver: &dyn LoggerResolver,
        trusted: &TrustedCallbacks,
    ) -> Result<(), DynamicCallbackError> {
        let Some(callbacks) = self.slot_mut(kind).take() else {
            return Ok(());
        };
        let mut processed = Vec::with_capacity(callbacks.len());
        for callback in callbacks {
            let DynamicCallback::Name(name) = &callback else {
                processed.push(callback);
                continue;
            };
            for logger in resolve::resolve_callback_string(name, trusted, resolver)? {
                let entry = DynamicCallback::Logger(logger);
                self.mirror_entry(kind, &entry);
                processed.push(entry);
            }
        }
        *self.slot_mut(kind) = Some(processed);
        Ok(())
    }

    fn mirror_entry(&mut self, kind: DynamicCallbackKind, entry: &DynamicCallback) {
        if let Some(mirror) = kind.mirror() {
            self.slot_mut(mirror)
                .get_or_insert_with(Vec::new)
                .push(entry.clone());
        }
    }

    const fn slot_mut(&mut self, kind: DynamicCallbackKind) -> &mut Option<Vec<DynamicCallback>> {
        match kind {
            DynamicCallbackKind::Input => &mut self.input,
            DynamicCallbackKind::Success => &mut self.success,
            DynamicCallbackKind::AsyncSuccess => &mut self.async_success,
            DynamicCallbackKind::Failure => &mut self.failure,
            DynamicCallbackKind::AsyncFailure => &mut self.async_failure,
        }
    }
}
