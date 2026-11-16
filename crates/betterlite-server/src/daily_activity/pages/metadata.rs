use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use std::collections::BTreeMap;

pub type MetadataFields = IndexMap<String, Json>;

pub type EntityMetadata = BTreeMap<String, MetadataFields>;
