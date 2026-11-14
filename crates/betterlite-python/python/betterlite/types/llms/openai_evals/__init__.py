from typing import ClassVar, Literal, Required, TypeAlias, TypedDict

from pydantic import BaseModel, ConfigDict, JsonValue


class _EvalsModel(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")


class DataSourceConfigCustom(TypedDict, total=False):
    type: Required[Literal["custom"]]
    item_schema: Required[dict[str, JsonValue]]
    include_sample_schema: bool | None


class DataSourceConfigLogs(TypedDict, total=False):
    type: Required[Literal["logs"]]
    metadata: dict[str, JsonValue] | None


class DataSourceConfigStoredCompletions(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    type: Required[Literal["stored_completions"]]
    metadata: dict[str, JsonValue] | None


DataSourceConfig: TypeAlias = (
    DataSourceConfigCustom
    | DataSourceConfigLogs
    | DataSourceConfigStoredCompletions  # ast-grep-ignore: no-long-names
)


class LLMAsJudgeGraderConfig(TypedDict, total=False):
    type: Required[Literal["llm_as_judge"]]
    model: str | None
    prompt: str | None


class GroundTruthGraderConfig(TypedDict, total=False):
    type: Required[Literal["ground_truth"]]
    metric: Literal["exact_match", "f1_score", "bleu"] | None


class CustomGraderConfig(TypedDict, total=False):
    type: Required[Literal["custom"]]
    function_id: Required[str]


GraderConfig: TypeAlias = (
    LLMAsJudgeGraderConfig | GroundTruthGraderConfig | CustomGraderConfig
)


class CreateEvalRequest(TypedDict, total=False):
    name: str | None
    data_source_config: Required[DataSourceConfig]
    testing_criteria: Required[list[GraderConfig]]
    metadata: dict[str, JsonValue] | None


class UpdateEvalRequest(TypedDict, total=False):
    name: str | None
    metadata: dict[str, JsonValue] | None


class ListEvalsParams(TypedDict, total=False):
    limit: int | None
    after: str | None
    before: str | None
    order: Literal["asc", "desc"] | None
    order_by: Literal["created_at", "updated_at"] | None


class Eval(_EvalsModel):
    id: str
    object: str = "eval"
    created_at: int
    updated_at: int | None = None
    name: str | None = None
    data_source_config: dict[str, JsonValue]
    testing_criteria: list[dict[str, JsonValue]]
    metadata: dict[str, JsonValue] | None = None


class ListEvalsResponse(_EvalsModel):
    object: str = "list"
    data: list[Eval]
    first_id: str | None = None
    last_id: str | None = None
    has_more: bool = False


class DeleteEvalResponse(_EvalsModel):
    eval_id: str
    object: str = "eval.deleted"
    deleted: bool


class CancelEvalResponse(_EvalsModel):
    id: str
    object: str = "eval"
    status: Literal["cancelled"]


class DataSourceDatasetConfig(TypedDict, total=False):
    type: Required[Literal["dataset"]]
    dataset_id: Required[str]


class DataSourceSampleSetConfig(TypedDict, total=False):
    type: Required[Literal["sample_set"]]
    sample_set_id: Required[str]


class DataSourceInlineConfig(TypedDict, total=False):
    type: Required[Literal["inline"]]
    samples: Required[list[dict[str, JsonValue]]]


RunDataSourceConfig: TypeAlias = (
    DataSourceDatasetConfig | DataSourceSampleSetConfig | DataSourceInlineConfig
)


class CompletionConfig(TypedDict, total=False):
    model: Required[str]
    temperature: float | None
    max_tokens: int | None
    top_p: float | None
    frequency_penalty: float | None
    presence_penalty: float | None


class CreateRunRequest(TypedDict, total=False):
    data_source: Required[dict[str, JsonValue]]
    name: str | None
    metadata: dict[str, JsonValue] | None


class ListRunsParams(TypedDict, total=False):
    limit: int | None
    after: str | None
    before: str | None
    order: Literal["asc", "desc"] | None


class ResultCounts(_EvalsModel):
    total: int
    passed: int = 0
    failed: int = 0
    error: int = 0


class PerTestingCriteriaResult(_EvalsModel):
    testing_criteria_index: int
    result_counts: ResultCounts
    average_score: float | None = None


class Run(_EvalsModel):
    id: str
    object: str = "eval.run"
    created_at: int
    status: Literal["queued", "running", "completed", "failed", "cancelled"]
    data_source: dict[str, JsonValue]
    eval_id: str
    name: str | None = None
    started_at: int | None = None
    completed_at: int | None = None
    model: str | None = None
    per_model_usage: JsonValue | None = None
    per_testing_criteria_results: list[PerTestingCriteriaResult] | None = None
    report_url: str | None = None
    result_counts: dict[str, int] | None = None
    shared_with_openai: bool | None = None
    metadata: dict[str, JsonValue] | None = None
    error: dict[str, JsonValue] | None = None


class ListRunsResponse(_EvalsModel):
    object: str = "list"
    data: list[Run]
    first_id: str | None = None
    last_id: str | None = None
    has_more: bool = False


class CancelRunResponse(_EvalsModel):
    id: str
    object: str = "eval.run"
    status: Literal["cancelled"]


class RunDeleteResponse(_EvalsModel):
    run_id: str
    object: str | None = "eval.run.deleted"
    deleted: bool | None = True
