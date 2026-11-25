mod answer;
mod world;

use betterlite_auth::auth_resource_grants::managed_rows::VectorStoreRow;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

use answer::answer;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

// the credential fields the assertions read, in the shape both sides build their auth object from
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoreAuth {
    #[serde(default)]
    pub(super) team_id: Option<String>,
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) user_role: Option<String>,
    #[serde(default)]
    pub(super) object_permission_id: Option<String>,
    #[serde(default)]
    pub(super) vector_stores: Option<Vec<String>>,
}

// the stores the scripted registry answers, matched on vector_store_id
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RegistryInput {
    #[serde(default)]
    pub(super) stores: Vec<LiteLlmManagedVectorStore>,
}

// the library call the case scripts a failure in
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    Registry,
    CacheRead,
    CacheWrite,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Call {
    Resolve {
        vector_store_id: String,
        #[serde(default)]
        registry: Option<RegistryInput>,
        database: bool,
        #[serde(default)]
        cached: Vec<VectorStoreRow>,
        #[serde(default)]
        rows: Vec<VectorStoreRow>,
        #[serde(default)]
        fail: Option<Failure>,
    },
    AssertStore {
        store: LiteLlmManagedVectorStore,
        auth: StoreAuth,
        #[serde(default)]
        detail: Option<String>,
    },
    AssertId {
        vector_store_id: String,
        #[serde(default)]
        registry: Option<RegistryInput>,
        database: bool,
        #[serde(default)]
        cached: Vec<VectorStoreRow>,
        #[serde(default)]
        rows: Vec<VectorStoreRow>,
        #[serde(default)]
        fail: Option<Failure>,
        auth: StoreAuth,
        #[serde(default)]
        detail: Option<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let answers = input
            .calls
            .iter()
            .map(|call| answer(&runtime, call))
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(Value::Array(answers))
    }
}
