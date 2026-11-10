mod classifier;
mod pipeline;
mod policy;
mod session;
mod storage;
mod support;
mod vector;

use serde_json::Value;

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    StoragePolicy(policy::StoragePolicySpec),
    BodyStorage(storage::BodyStorageSpec),
    Classifier(classifier::ClassifierSpec),
    VectorAndPlaceholder(vector::VectorSpec),
    Session(session::SessionSpec),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotSpec {
    pub(super) value: Value,
    pub(super) openai_sdk: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DurationSpec {
    pub(super) start: String,
    pub(super) end: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::StoragePolicy(spec) => policy::storage_policy(spec),
            Input::BodyStorage(spec) => storage::body_storage(spec),
            Input::Classifier(spec) => classifier::classifier(spec),
            Input::VectorAndPlaceholder(spec) => vector::vector_and_placeholder(spec),
            Input::Session(spec) => session::session(spec),
        }
    }
}
