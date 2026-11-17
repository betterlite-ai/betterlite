use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::data_cloud_dispatch::context::DispatchGlobals;

#[derive(Clone)]
pub(super) struct OwnedGlobals {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) databricks_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
}

impl OwnedGlobals {
    pub(super) const fn empty() -> Self {
        Self {
            api_base: None,
            api_key: None,
            databricks_key: None,
            headers: None,
        }
    }

    pub(super) fn borrowed(&self) -> DispatchGlobals<'_> {
        DispatchGlobals {
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            databricks_key: self.databricks_key.as_ref(),
            headers: self.headers.as_ref(),
        }
    }
}
