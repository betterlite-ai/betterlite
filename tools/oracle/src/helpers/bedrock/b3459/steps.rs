use std::collections::BTreeMap;

use betterlite_config::rag::document::ingest::{
    RAGIngestEmbeddingOptions, S3VectorsVectorStoreOptions,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_rag::s3_vectors_workflow::settings::S3VectorsSettings;
use betterlite_rag::s3_vectors_workflow::workflow::{
    NO_CONTENT_MESSAGE, S3VectorsWorkflow, WorkflowError,
};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::recorder::{Recorder, RecordingApi, ScriptedApiError, ScriptedEmbedder};
use super::{DimensionCase, EmbedStep, FlowCase, SettingsCase, StoreCall, StoreStep};

const UNIQUE_ID: &str = "0a1b2c3d";

type StoreOutcome = Result<(Option<String>, Option<String>), WorkflowError<ScriptedApiError>>;

struct CaseSettings<'case> {
    vector: &'case S3VectorsVectorStoreOptions,
    environment: &'case BTreeMap<String, String>,
    embedding: Option<RAGIngestEmbeddingOptions>,
}

fn build_workflow(
    case: &CaseSettings<'_>,
    recorder: &Recorder,
    answer: Vec<Vec<f64>>,
    failing: bool,
) -> Result<S3VectorsWorkflow<RecordingApi, ScriptedEmbedder>, Box<HelperError>> {
    let settings = S3VectorsSettings::from_vector_store_config(
        case.vector,
        case.embedding.clone(),
        case.environment,
    )
    .or_value_error()?;
    let embedder = ScriptedEmbedder::new(recorder.clone(), answer, failing);
    Ok(S3VectorsWorkflow::new(
        settings,
        RecordingApi::new(recorder.clone()),
        embedder,
        UNIQUE_ID.to_owned(),
    ))
}

pub(super) fn settings(cases: &[SettingsCase]) -> HelperResult {
    let mut results: Vec<Value> = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(settings_for(case));
    }
    Ok(json!({"settings": results}))
}

fn settings_for(case: &SettingsCase) -> Value {
    let built = S3VectorsSettings::from_vector_store_config(
        &case.vector,
        case.embedding.clone(),
        &case.environment,
    );
    match built {
        Ok(settings) => json!({
            "vector_bucket_name": settings.vector_bucket_name,
            "index_name": settings.index_name,
            "embedding_model": settings.embedding_model,
            "distance_metric": settings.distance_metric,
            "non_filterable_metadata_keys": settings.non_filterable_metadata_keys,
            "dimension": settings.dimension,
            "aws_region_name": settings.aws_region_name,
        }),
        Err(error) => json!({"failed": error.to_string()}),
    }
}

pub(super) fn dimension(runtime: &Runtime, cases: &[DimensionCase]) -> HelperResult {
    let mut results: Vec<Value> = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(dimension_for(runtime, case)?);
    }
    Ok(json!({"cases": results}))
}

fn dimension_for(runtime: &Runtime, case: &DimensionCase) -> HelperResult {
    let recorder = Recorder::default();
    let settings = CaseSettings {
        vector: &case.vector,
        environment: &case.environment,
        embedding: case.embedding.clone(),
    };
    let workflow = build_workflow(&settings, &recorder, case.answer.clone(), case.failing)?;
    let chunks = vec!["one".to_owned()];
    let embeddings = vec![vec![0.5_f64]];
    runtime
        .block_on(workflow.store(Some("a.txt"), &chunks, Some(embeddings.as_slice())))
        .or_value_error()?;
    Ok(json!({"operations": recorder.operations()}))
}

pub(super) fn flow(runtime: &Runtime, cases: &[FlowCase]) -> HelperResult {
    let mut results: Vec<Value> = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(flow_for(runtime, case)?);
    }
    Ok(json!({"cases": results}))
}

fn flow_for(runtime: &Runtime, case: &FlowCase) -> HelperResult {
    let recorder = Recorder::default();
    let settings = CaseSettings {
        vector: &case.vector,
        environment: &case.environment,
        embedding: case.embedding.clone(),
    };
    let workflow = build_workflow(&settings, &recorder, case.answer.clone(), false)?;
    let outcome = runtime.block_on(workflow.store(
        case.filename.as_deref(),
        &case.chunks,
        Some(case.embeddings.as_slice()),
    ));
    Ok(json!({
        "result": result_value(outcome)?,
        "operations": recorder.operations(),
    }))
}

pub(super) fn embed(runtime: &Runtime, step: &EmbedStep) -> HelperResult {
    let recorder = Recorder::default();
    let settings = CaseSettings {
        vector: &step.vector,
        environment: &step.environment,
        embedding: step.embedding.clone(),
    };
    let workflow = build_workflow(&settings, &recorder, step.answer.clone(), false)?;
    let mut results: Vec<Value> = Vec::with_capacity(step.chunk_sets.len());
    for chunks in &step.chunk_sets {
        let vectors = runtime.block_on(workflow.embed(chunks)).or_value_error()?;
        results.push(json!(vectors));
    }
    Ok(json!({
        "results": results,
        "operations": recorder.operations(),
    }))
}

pub(super) fn store(runtime: &Runtime, step: &StoreStep) -> HelperResult {
    let recorder = Recorder::default();
    let settings = CaseSettings {
        vector: &step.vector,
        environment: &step.environment,
        embedding: step.embedding.clone(),
    };
    let workflow = build_workflow(&settings, &recorder, Vec::new(), false)?;
    let mut results: Vec<Value> = Vec::with_capacity(step.calls.len());
    for call in &step.calls {
        results.push(store_call(&workflow, runtime, call)?);
    }
    Ok(json!({
        "results": results,
        "operations": recorder.operations(),
    }))
}

fn store_call(
    workflow: &S3VectorsWorkflow<RecordingApi, ScriptedEmbedder>,
    runtime: &Runtime,
    call: &StoreCall,
) -> HelperResult {
    let outcome = runtime.block_on(workflow.store(
        call.filename.as_deref(),
        &call.chunks,
        call.embeddings.as_deref(),
    ));
    result_value(outcome)
}

fn result_value(outcome: StoreOutcome) -> HelperResult {
    match outcome {
        Ok((vector_store_id, filename)) => {
            Ok(json!({"vector_store_id": vector_store_id, "filename": filename}))
        }
        Err(WorkflowError::NoContent) => Ok(json!({"failed": NO_CONTENT_MESSAGE})),
        Err(WorkflowError::Embed(error)) => Err::<Value, _>(error).or_value_error(),
        Err(WorkflowError::Api(error)) => Err::<Value, _>(error).or_value_error(),
    }
}
