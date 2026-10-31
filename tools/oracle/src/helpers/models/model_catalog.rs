use crate::helpers::{HelperError, HelperResult};
use betterlite_models::catalog::{
    BundledCatalog, CatalogError, ModelCatalog, ModelMetadataProjection, UnknownFields,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{io, sync::Arc};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    source: Option<Box<RawValue>>,
    #[serde(default)]
    lookup: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    PackagedMap,
    BundledCatalogReadFailure,
    CanonicalLookup,
    Aliases,
    AliasCanonicalConflict,
    AliasAliasConflict,
    GeneralizationsExcluded,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.operation {
            Operation::PackagedMap => {
                let mut loader = BundledCatalog::default();
                let catalog = loader.get();
                Ok(json!({"count": catalog.entries.len(), "warnings": catalog.warnings}))
            }
            Operation::BundledCatalogReadFailure => Ok(bundled_failure()),
            Operation::CanonicalLookup
            | Operation::Aliases
            | Operation::AliasCanonicalConflict
            | Operation::AliasAliasConflict
            | Operation::GeneralizationsExcluded => summarize_source(input),
        }
    }
}

fn bundled_failure() -> Value {
    let reads = AtomicUsize::new(0);
    let mut loader = BundledCatalog {
        cached: None,
        asset: || {
            reads.fetch_add(1, Ordering::SeqCst);
            Err(CatalogError::Asset(io::Error::from(
                io::ErrorKind::NotFound,
            )))
        },
    };
    let first = loader.get();
    let cached = Arc::ptr_eq(&first, &loader.get());
    json!({"catalog": first.lookup, "count": first.entries.len(), "warnings": first.warnings,
        "cached": cached, "reads": reads.load(Ordering::SeqCst)})
}

fn summarize_source(input: &Input) -> HelperResult {
    let source = input.source.as_ref().ok_or_else(|| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "source is required".to_owned(),
        })
    })?;
    let catalog =
        ModelCatalog::parse(source.get().as_bytes(), UnknownFields::Warn).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    let summary = Summary {
        warnings: &catalog.warnings,
        warning_count: catalog.warnings.len(),
        lookup: input
            .lookup
            .iter()
            .map(|name| catalog.metadata(name))
            .collect(),
        count: matches!(input.operation, Operation::GeneralizationsExcluded)
            .then_some(catalog.entries.len()),
    };
    Ok(json!(summary))
}

#[derive(Serialize)]
struct Summary<'catalog> {
    warnings: &'catalog [String],
    warning_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lookup: Vec<Option<ModelMetadataProjection<'catalog>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
}

mod tests;
