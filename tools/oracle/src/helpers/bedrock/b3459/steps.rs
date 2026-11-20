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

type BuiltWorkflow = (S3VectorsWorkflow<RecordingApi, ScriptedEmbedder>, Recorder);

trait StepSource {
    fn vector(&self) -> &S3VectorsVectorStoreOptions;
    fn environment(&self) -> &BTreeMap<String, String>;
    fn embedding(&self) -> Option<RAGIngestEmbeddingOptions>;
    fn answer(&self) -> Vec<Vec<f64>>;
}

impl StepSource for DimensionCase {
    fn vector(&self) -> &S3VectorsVectorStoreOptions {
        &self.vector
    }

    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    fn embedding(&self) -> Option<RAGIngestEmbeddingOptions> {
        self.embedding.clone()
    }

    fn answer(&self) -> Vec<Vec<f64>> {
        self.answer.clone()
    }
}

impl StepSource for FlowCase {
    fn vector(&self) -> &S3VectorsVectorStoreOptions {
        &self.vector
    }

    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    fn embedding(&self) -> Option<RAGIngestEmbeddingOptions> {
        self.embedding.clone()
    }

    fn answer(&self) -> Vec<Vec<f64>> {
        self.answer.clone()
    }
}

impl StepSource for EmbedStep {
    fn vector(&self) -> &S3VectorsVectorStoreOptions {
        &self.vector
    }

    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    fn embedding(&self) -> Option<RAGIngestEmbeddingOptions> {
        self.embedding.clone()
    }

    fn answer(&self) -> Vec<Vec<f64>> {
        self.answer.clone()
    }
}

impl StepSource for StoreStep {
    fn vector(&self) -> &S3VectorsVectorStoreOptions {
        &self.vector
    }

    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    fn embedding(&self) -> Option<RAGIngestEmbeddingOptions> {
        self.embedding.clone()
    }

    fn answer(&self) -> Vec<Vec<f64>> {
        Vec::new()
    }
}

fn build_workflow<S: StepSource>(
    source: &S,
    failing: bool,
) -> Result<BuiltWorkflow, Box<HelperError>> {
    let recorder = Recorder::default();
    let settings = S3VectorsSettings::from_vector_store_config(
        source.vector(),
        source.embedding(),
        source.environment(),
    )
    .or_value_error()?;
    let embedder = ScriptedEmbedder::new(recorder.clone(), source.answer(), failing);
    let workflow = S3VectorsWorkflow::new(
        settings,
        RecordingApi {
            recorder: recorder.clone(),
        },
        embedder,
        UNIQUE_ID.to_owned(),
    );
    Ok((workflow, recorder))
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
    let (workflow, recorder) = build_workflow(case, case.failing)?;
    let chunks = vec!["one".to_owned()];
    let embeddings = vec![vec![0.5_f64]];
    runtime
        .block_on(workflow.store_vectors(Some("a.txt"), &chunks, Some(embeddings.as_slice())))
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
    let (workflow, recorder) = build_workflow(case, false)?;
    let outcome = runtime.block_on(workflow.store_vectors(
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
    let (workflow, recorder) = build_workflow(step, false)?;
    let results = embed_chunks(&workflow, runtime, &step.chunk_sets)?;
    results_value(results, &recorder)
}

pub(super) fn store(runtime: &Runtime, step: &StoreStep) -> HelperResult {
    let (workflow, recorder) = build_workflow(step, false)?;
    let results = store_calls(&workflow, runtime, &step.calls)?;
    results_value(results, &recorder)
}

fn embed_chunks(
    workflow: &S3VectorsWorkflow<RecordingApi, ScriptedEmbedder>,
    runtime: &Runtime,
    chunk_sets: &[Vec<String>],
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results: Vec<Value> = Vec::with_capacity(chunk_sets.len());
    for chunks in chunk_sets {
        let vectors = runtime.block_on(workflow.embed(chunks)).or_value_error()?;
        results.push(json!(vectors));
    }
    Ok(results)
}

fn store_calls(
    workflow: &S3VectorsWorkflow<RecordingApi, ScriptedEmbedder>,
    runtime: &Runtime,
    calls: &[StoreCall],
) -> Result<Vec<Value>, Box<HelperError>> {
    calls
        .iter()
        .map(|call| store_call(workflow, runtime, call))
        .collect()
}

fn results_value(results: Vec<Value>, recorder: &Recorder) -> HelperResult {
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
    result_value(runtime.block_on(workflow.store_vectors(
        call.filename.as_deref(),
        &call.chunks,
        call.embeddings.as_deref(),
    )))
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
