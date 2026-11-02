#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::{
    CancelEvalResponse, CancelRunResponse, CancelledStatus, CreateEvalRequest, CreateRunRequest,
    CustomGraderConfig, DataSourceConfig, DataSourceConfigCustom, DataSourceConfigLogs,
    DataSourceStoredCompletions, DeleteEvalResponse, Eval, EvalOrder, EvalOrderBy, GraderConfig,
    GroundTruthGraderConfig, GroundTruthMetric, LLMAsJudgeGraderConfig, ListEvalsParams,
    ListEvalsResponse, ListRunsParams, ListRunsResponse, PerTestingCriteriaResult, ResultCounts,
    Run, RunDeleteResponse, RunDeleted, RunDeletedObject, RunStatus, UpdateEvalRequest,
};

#[test]
fn data_source_config_custom_requires_item_schema_and_defaults_sample_schema() {
    let config: DataSourceConfig =
        serde_json::from_str(r#"{"type": "custom", "item_schema": {"role": "string"}}"#).unwrap();
    assert_eq!(
        config,
        DataSourceConfig::Custom(DataSourceConfigCustom {
            item_schema: serde_json::from_str(r#"{"role": "string"}"#).unwrap(),
            include_sample_schema: None,
        })
    );
    serde_json::from_str::<DataSourceConfig>(r#"{"type": "custom"}"#).unwrap_err();
}

#[test]
fn data_source_config_custom_omits_absent_include_sample_schema_on_the_wire() {
    let config = DataSourceConfig::Custom(DataSourceConfigCustom {
        item_schema: serde_json::from_str(r#"{"role": "string"}"#).unwrap(),
        include_sample_schema: None,
    });
    assert_eq!(
        serde_json::to_value(&config).unwrap(),
        serde_json::from_str::<Value>(r#"{"type": "custom", "item_schema": {"role": "string"}}"#)
            .unwrap()
    );
}

#[test]
fn data_source_config_logs_accepts_optional_metadata() {
    let config: DataSourceConfig =
        serde_json::from_str(r#"{"type": "logs", "metadata": {"run": "a"}}"#).unwrap();
    assert_eq!(
        config,
        DataSourceConfig::Logs(DataSourceConfigLogs {
            metadata: serde_json::from_str(r#"{"run": "a"}"#).unwrap(),
        })
    );
}

#[test]
fn data_source_config_stored_completions_accepts_optional_metadata() {
    let config: DataSourceConfig =
        serde_json::from_str(r#"{"type": "stored_completions"}"#).unwrap();
    assert_eq!(
        config,
        DataSourceConfig::StoredCompletions(DataSourceStoredCompletions { metadata: None })
    );
}

#[test]
fn grader_config_llm_as_judge_holds_optional_model_and_prompt() {
    let grader: GraderConfig = serde_json::from_str(
        r#"{"type": "llm_as_judge", "model": "gpt-4", "prompt": "grade this"}"#,
    )
    .unwrap();
    assert_eq!(
        grader,
        GraderConfig::LlmAsJudge(LLMAsJudgeGraderConfig {
            model: Some("gpt-4".to_owned()),
            prompt: Some("grade this".to_owned()),
        })
    );
}

#[test]
fn grader_config_llm_as_judge_omits_absent_model_and_prompt_on_the_wire() {
    let grader = GraderConfig::LlmAsJudge(LLMAsJudgeGraderConfig {
        model: None,
        prompt: None,
    });
    assert_eq!(
        serde_json::to_value(&grader).unwrap(),
        serde_json::from_str::<Value>(r#"{"type": "llm_as_judge"}"#).unwrap()
    );
}

#[test]
fn grader_config_ground_truth_holds_optional_metric() {
    let grader: GraderConfig =
        serde_json::from_str(r#"{"type": "ground_truth", "metric": "f1_score"}"#).unwrap();
    assert_eq!(
        grader,
        GraderConfig::GroundTruth(GroundTruthGraderConfig {
            metric: Some(GroundTruthMetric::F1Score),
        })
    );
    let bare: GraderConfig = serde_json::from_str(r#"{"type": "ground_truth"}"#).unwrap();
    assert_eq!(
        bare,
        GraderConfig::GroundTruth(GroundTruthGraderConfig { metric: None })
    );
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::from_str::<Value>(r#"{"type": "ground_truth"}"#).unwrap()
    );
}

#[rstest]
#[case::exact_match("exact_match", GroundTruthMetric::ExactMatch)]
#[case::f1_score("f1_score", GroundTruthMetric::F1Score)]
#[case::bleu("bleu", GroundTruthMetric::Bleu)]
fn ground_truth_metric_covers_every_wire_value(
    #[case] wire: &str,
    #[case] metric: GroundTruthMetric,
) {
    let quoted = format!("\"{wire}\"");
    assert_eq!(
        serde_json::from_str::<GroundTruthMetric>(&quoted).unwrap(),
        metric
    );
}

#[test]
fn grader_config_custom_requires_function_id() {
    let grader: GraderConfig =
        serde_json::from_str(r#"{"type": "custom", "function_id": "fn-1"}"#).unwrap();
    assert_eq!(
        grader,
        GraderConfig::Custom(CustomGraderConfig {
            function_id: "fn-1".to_owned(),
        })
    );
}

#[test]
fn create_eval_request_requires_data_source_config_and_testing_criteria() {
    let request: CreateEvalRequest = serde_json::from_str(
        r#"{"data_source_config": {"type": "logs"}, "testing_criteria": [{"type": "custom", "function_id": "fn-1"}]}"#,
    )
    .unwrap();
    assert_eq!(request.name, None);
    assert_eq!(
        request.data_source_config,
        DataSourceConfig::Logs(DataSourceConfigLogs { metadata: None })
    );
    assert_eq!(request.testing_criteria.len(), 1);
    assert_eq!(request.metadata, None);
    serde_json::from_str::<CreateEvalRequest>(r#"{"data_source_config": {"type": "logs"}}"#)
        .unwrap_err();
    serde_json::from_str::<CreateEvalRequest>(
        r#"{"testing_criteria": [{"type": "custom", "function_id": "fn-1"}]}"#,
    )
    .unwrap_err();
}

#[test]
fn create_eval_request_omits_absent_name_and_metadata_on_the_wire() {
    let request = CreateEvalRequest {
        name: None,
        data_source_config: DataSourceConfig::Logs(DataSourceConfigLogs { metadata: None }),
        testing_criteria: vec![GraderConfig::Custom(CustomGraderConfig {
            function_id: "fn-1".to_owned(),
        })],
        metadata: None,
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::from_str::<Value>(
            r#"{"data_source_config": {"type": "logs"}, "testing_criteria": [{"type": "custom", "function_id": "fn-1"}]}"#
        )
        .unwrap()
    );
}

#[test]
fn create_eval_request_omits_every_absent_nested_optional_on_the_wire() {
    let request = CreateEvalRequest {
        name: None,
        data_source_config: DataSourceConfig::Custom(DataSourceConfigCustom {
            item_schema: serde_json::from_str(r#"{"role": "string"}"#).unwrap(),
            include_sample_schema: None,
        }),
        testing_criteria: vec![
            GraderConfig::LlmAsJudge(LLMAsJudgeGraderConfig {
                model: None,
                prompt: None,
            }),
            GraderConfig::GroundTruth(GroundTruthGraderConfig { metric: None }),
        ],
        metadata: None,
    };
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(
        wire,
        serde_json::from_str::<Value>(
            r#"{"data_source_config": {"type": "custom", "item_schema": {"role": "string"}}, "testing_criteria": [{"type": "llm_as_judge"}, {"type": "ground_truth"}]}"#
        )
        .unwrap()
    );
    let wire_text = wire.to_string();
    assert!(!wire_text.contains("include_sample_schema"));
    assert!(!wire_text.contains("model"));
    assert!(!wire_text.contains("prompt"));
    assert!(!wire_text.contains("metric"));
    assert!(!wire_text.contains("name"));
    assert!(!wire_text.contains("metadata"));
}

#[test]
fn update_eval_request_fields_are_all_optional() {
    let request: UpdateEvalRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(request.name, None);
    assert_eq!(request.metadata, None);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::from_str::<Value>("{}").unwrap()
    );
}

#[test]
fn list_evals_params_fields_are_all_optional() {
    let params: ListEvalsParams =
        serde_json::from_str(r#"{"limit": 10, "order": "asc", "order_by": "updated_at"}"#).unwrap();
    assert_eq!(params.limit, Some(10));
    assert_eq!(params.after, None);
    assert_eq!(params.before, None);
    assert_eq!(params.order, Some(EvalOrder::Asc));
    assert_eq!(params.order_by, Some(EvalOrderBy::UpdatedAt));
    let bare: ListEvalsParams = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::from_str::<Value>("{}").unwrap()
    );
}

#[test]
fn eval_object_defaults_to_eval_literal() {
    let value: Eval = serde_json::from_str(
        r#"{"id": "eval-1", "created_at": 1, "data_source_config": {"type": "logs"}, "testing_criteria": []}"#,
    )
    .unwrap();
    assert_eq!(value.object, "eval");
    assert_eq!(value.updated_at, None);
    assert_eq!(value.name, None);
    assert_eq!(value.metadata, None);
}

#[test]
fn list_evals_response_defaults_object_and_has_more() {
    let response: ListEvalsResponse = serde_json::from_str(r#"{"data": []}"#).unwrap();
    assert_eq!(response.object, "list");
    assert!(!response.has_more);
    assert_eq!(response.first_id, None);
    assert_eq!(response.last_id, None);
}

#[test]
fn delete_eval_response_requires_deleted_and_defaults_object() {
    let response: DeleteEvalResponse =
        serde_json::from_str(r#"{"eval_id": "eval-1", "deleted": true}"#).unwrap();
    assert_eq!(response.object, "eval.deleted");
    assert!(response.deleted);
    serde_json::from_str::<DeleteEvalResponse>(r#"{"eval_id": "eval-1"}"#).unwrap_err();
}

#[test]
fn cancel_eval_response_requires_cancelled_status_and_defaults_object() {
    let response: CancelEvalResponse =
        serde_json::from_str(r#"{"id": "eval-1", "status": "cancelled"}"#).unwrap();
    assert_eq!(response.object, "eval");
    assert_eq!(response.status, CancelledStatus::Cancelled);
}

#[test]
fn create_run_request_requires_data_source() {
    let request: CreateRunRequest =
        serde_json::from_str(r#"{"data_source": {"type": "logs"}}"#).unwrap();
    assert_eq!(request.name, None);
    assert_eq!(request.metadata, None);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::from_str::<Value>(r#"{"data_source": {"type": "logs"}}"#).unwrap()
    );
    serde_json::from_str::<CreateRunRequest>("{}").unwrap_err();
}

#[test]
fn list_runs_params_fields_are_all_optional_and_has_no_order_by() {
    let params: ListRunsParams = serde_json::from_str(r#"{"order": "desc"}"#).unwrap();
    assert_eq!(params.limit, None);
    assert_eq!(params.order, Some(EvalOrder::Desc));
    let bare: ListRunsParams = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::from_str::<Value>("{}").unwrap()
    );
}

#[test]
fn result_counts_passed_failed_and_error_default_to_zero() {
    let counts: ResultCounts = serde_json::from_str(r#"{"total": 5}"#).unwrap();
    assert_eq!(counts.passed, 0);
    assert_eq!(counts.failed, 0);
    assert_eq!(counts.error, 0);
}

#[test]
fn per_testing_criteria_result_holds_counts_and_optional_average_score() {
    let result: PerTestingCriteriaResult = serde_json::from_str(
        r#"{"testing_criteria_index": 0, "result_counts": {"total": 1, "passed": 1}}"#,
    )
    .unwrap();
    assert_eq!(result.result_counts.total, 1);
    assert_eq!(result.average_score, None);
}

#[rstest]
#[case::queued("queued", RunStatus::Queued)]
#[case::running("running", RunStatus::Running)]
#[case::completed("completed", RunStatus::Completed)]
#[case::failed("failed", RunStatus::Failed)]
#[case::cancelled("cancelled", RunStatus::Cancelled)]
fn run_status_covers_every_lifecycle_state(#[case] wire: &str, #[case] status: RunStatus) {
    let quoted = format!("\"{wire}\"");
    assert_eq!(serde_json::from_str::<RunStatus>(&quoted).unwrap(), status);
}

#[test]
fn run_defaults_object_and_leaves_optional_fields_absent() {
    let run: Run = serde_json::from_str(
        r#"{"id": "run-1", "created_at": 1, "status": "queued", "data_source": {"type": "logs"}, "eval_id": "eval-1"}"#,
    )
    .unwrap();
    assert_eq!(run.object, "eval.run");
    assert_eq!(run.started_at, None);
    assert_eq!(run.completed_at, None);
    assert_eq!(run.model, None);
    assert_eq!(run.per_model_usage, None);
    assert_eq!(run.per_testing_criteria_results, None);
    assert_eq!(run.report_url, None);
    assert_eq!(run.result_counts, None);
    assert_eq!(run.shared_with_openai, None);
    assert_eq!(run.metadata, None);
    assert_eq!(run.error, None);
}

#[test]
fn list_runs_response_defaults_object_and_has_more() {
    let response: ListRunsResponse = serde_json::from_str(r#"{"data": []}"#).unwrap();
    assert_eq!(response.object, "list");
    assert!(!response.has_more);
}

#[test]
fn cancel_run_response_requires_cancelled_status_and_defaults_object() {
    let response: CancelRunResponse =
        serde_json::from_str(r#"{"id": "run-1", "status": "cancelled"}"#).unwrap();
    assert_eq!(response.object, "eval.run");
    assert_eq!(response.status, CancelledStatus::Cancelled);
}

#[test]
fn run_delete_response_defaults_object_and_deleted_when_omitted() {
    let response: RunDeleteResponse = serde_json::from_str(r#"{"run_id": "run-1"}"#).unwrap();
    assert_eq!(
        response.object,
        RunDeletedObject(Some("eval.run.deleted".to_owned()))
    );
    assert_eq!(response.deleted, RunDeleted(Some(true)));
}

#[test]
fn run_delete_response_keeps_explicit_null_as_none() {
    let response: RunDeleteResponse =
        serde_json::from_str(r#"{"run_id": "run-1", "object": null, "deleted": null}"#).unwrap();
    assert_eq!(response.object, RunDeletedObject(None));
    assert_eq!(response.deleted, RunDeleted(None));
}
