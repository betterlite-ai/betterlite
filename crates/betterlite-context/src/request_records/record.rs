use std::collections::BTreeMap;
#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum HandleKind { Exception, Callable, Logging, LiteLLMLoggingObj, Router, Opaque }

#[derive(Debug, Clone, PartialEq, Eq)]
#[rustfmt::skip]
pub struct InternalHandle { pub kind: HandleKind, pub identity: u64 }

impl InternalHandle {
    #[must_use]
    pub const fn new(kind: HandleKind, identity_value: u64) -> Self {
        let identity = identity_value.saturating_add(0);
        Self { kind, identity }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
#[rustfmt::skip]
pub enum RequestRecord { Null, Bool(bool), Signed(i64), Unsigned(u64), Float(f64), String(String), Array(Vec<Self>), Object(BTreeMap<String, Self>), #[serde(skip)] Handle(InternalHandle) }

impl RequestRecord {
    pub(super) const fn object(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Object(fields) = self {
            Some(fields)
        } else {
            None
        }
    }

    pub(super) const fn object_mut(&mut self) -> Option<&mut BTreeMap<String, Self>> {
        if let Self::Object(fields) = self {
            Some(fields)
        } else {
            None
        }
    }
}
