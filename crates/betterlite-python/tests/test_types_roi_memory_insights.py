import math
from collections.abc import Sequence
from dataclasses import dataclass
from importlib import import_module
from typing import NotRequired, Protocol

import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from typing_extensions import TypedDict, TypeForm

from betterlite.types import memory_management, model_insights, roi_calculator

# litellm/types has no __init__.py, so pyright finds no py.typed for a static import from it
LITELLM_ROI = import_module("litellm.types.roi_calculator")
LITELLM_MEMORY = import_module("litellm.types.memory_management")
LITELLM_INSIGHTS = import_module("litellm.types.model_insights")


class LiteLLMRoi:
    DEFAULT_PROMPT: str = LITELLM_ROI.DEFAULT_PROMPT
    ROISettings: type[BaseModel] = LITELLM_ROI.ROISettings
    ROISettingsUpdate: type[BaseModel] = LITELLM_ROI.ROISettingsUpdate
    ROISettingsResponse: type[BaseModel] = LITELLM_ROI.ROISettingsResponse
    ROIRepository: type[BaseModel] = LITELLM_ROI.ROIRepository
    ROIRepositoriesResponse: type[BaseModel] = LITELLM_ROI.ROIRepositoriesResponse
    ROISyncStatus: type[BaseModel] = LITELLM_ROI.ROISyncStatus
    ROISpendRecord: TypeForm[roi_calculator.ROISpendRecord] = LITELLM_ROI.ROISpendRecord
    ROIEstimate: TypeForm[roi_calculator.ROIEstimate] = LITELLM_ROI.ROIEstimate
    ROIPullRecord: TypeForm[roi_calculator.ROIPullRecord] = LITELLM_ROI.ROIPullRecord
    ROIReport: TypeForm[roi_calculator.ROIReport] = LITELLM_ROI.ROIReport
    ROIPullFile: TypeForm[roi_calculator.ROIPullFile] = LITELLM_ROI.ROIPullFile
    ROIPullCommit: TypeForm[roi_calculator.ROIPullCommit] = LITELLM_ROI.ROIPullCommit
    ROIPullEvidence: TypeForm[roi_calculator.ROIPullEvidence] = LITELLM_ROI.ROIPullEvidence
    ROIIdentityMatch: TypeForm[roi_calculator.ROIIdentityMatch] = LITELLM_ROI.ROIIdentityMatch
    ROIPersonSummary: TypeForm[roi_calculator.ROIPersonSummary] = LITELLM_ROI.ROIPersonSummary
    ROIPullSummary: TypeForm[roi_calculator.ROIPullSummary] = LITELLM_ROI.ROIPullSummary
    ROISummaryMetrics: TypeForm[roi_calculator.ROISummaryMetrics] = LITELLM_ROI.ROISummaryMetrics
    ROITrendDay: TypeForm[roi_calculator.ROITrendDay] = LITELLM_ROI.ROITrendDay
    ROISummary: TypeForm[roi_calculator.ROISummary] = LITELLM_ROI.ROISummary
    ROIMetricsResponse: type[BaseModel] = LITELLM_ROI.ROIMetricsResponse
    ROIPersonResponse: type[BaseModel] = LITELLM_ROI.ROIPersonResponse
    ROIEstimateResponse: type[BaseModel] = LITELLM_ROI.ROIEstimateResponse
    ROIPullResponse: type[BaseModel] = LITELLM_ROI.ROIPullResponse
    ROITrendResponse: type[BaseModel] = LITELLM_ROI.ROITrendResponse
    ROISummaryResponse: type[BaseModel] = LITELLM_ROI.ROISummaryResponse
    ROIReportResponse: type[BaseModel] = LITELLM_ROI.ROIReportResponse
    ROIIdentityMapUpdate: type[BaseModel] = LITELLM_ROI.ROIIdentityMapUpdate
    ROIIdentityMapResponse: type[BaseModel] = LITELLM_ROI.ROIIdentityMapResponse
    ROIEstimatorChanges: type[BaseModel] = LITELLM_ROI.ROIEstimatorChanges
    ROIEstimatorFile: type[BaseModel] = LITELLM_ROI.ROIEstimatorFile
    ROIEstimatorCommit: type[BaseModel] = LITELLM_ROI.ROIEstimatorCommit
    ROIEstimatorEvidence: type[BaseModel] = LITELLM_ROI.ROIEstimatorEvidence
    ROICompletionMessage: TypeForm[roi_calculator.ROICompletionMessage] = (
        LITELLM_ROI.ROICompletionMessage
    )
    ROICompletionMetadata: TypeForm[roi_calculator.ROICompletionMetadata] = (
        LITELLM_ROI.ROICompletionMetadata
    )
    ROIResponseFormat: TypeForm[roi_calculator.ROIResponseFormat] = LITELLM_ROI.ROIResponseFormat
    ROICompletionRequest: type[BaseModel] = LITELLM_ROI.ROICompletionRequest
    ROICompletionResponse: type[BaseModel] = LITELLM_ROI.ROICompletionResponse
    ROIEstimatorResult: type[BaseModel] = LITELLM_ROI.ROIEstimatorResult


class LiteLLMMemory:
    LiteLLM_MemoryRow: type[BaseModel] = LITELLM_MEMORY.LiteLLM_MemoryRow
    MemoryCreateRequest: type[BaseModel] = LITELLM_MEMORY.MemoryCreateRequest
    MemoryUpdateRequest: type[BaseModel] = LITELLM_MEMORY.MemoryUpdateRequest
    MemoryListResponse: type[BaseModel] = LITELLM_MEMORY.MemoryListResponse
    MemoryDeleteResponse: type[BaseModel] = LITELLM_MEMORY.MemoryDeleteResponse


class LiteLLMInsights:
    ModelInsightsMetric: TypeForm[model_insights.ModelInsightsMetric] = (
        LITELLM_INSIGHTS.ModelInsightsMetric
    )
    ModelInsightMetric: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightMetric
    ModelInsightDailyMetric: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightDailyMetric
    ModelInsightDailyTotal: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightDailyTotal
    ModelInsightTask: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightTask
    ModelInsightTaskSummary: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightTaskSummary
    ModelInsightsResponse: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightsResponse
    ModelInsightTasksResponse: type[BaseModel] = LITELLM_INSIGHTS.ModelInsightTasksResponse


class SchemaSource(Protocol):
    def json_schema(self) -> dict[str, JsonValue]: ...


class ObjectSchema(TypedDict):
    properties: dict[str, JsonValue]
    required: NotRequired[list[str]]


class FieldSchema(TypedDict):
    default: NotRequired[JsonValue]
    description: NotRequired[str]


class ErrorShape(TypedDict):
    type: str
    loc: tuple[int | str, ...]
    msg: str


@dataclass(frozen=True)
class CompletionMessage:
    content: str | None


@dataclass(frozen=True)
class CompletionChoice:
    finish_reason: str | None
    message: CompletionMessage


@dataclass(frozen=True)
class Completion:
    choices: tuple[CompletionChoice, ...]


MODEL_PAIRS: tuple[tuple[type[BaseModel], type[BaseModel]], ...] = (
    (roi_calculator.ROISettings, LiteLLMRoi.ROISettings),
    (roi_calculator.ROISettingsUpdate, LiteLLMRoi.ROISettingsUpdate),
    (roi_calculator.ROISettingsResponse, LiteLLMRoi.ROISettingsResponse),
    (roi_calculator.ROIRepository, LiteLLMRoi.ROIRepository),
    (roi_calculator.ROIRepositoriesResponse, LiteLLMRoi.ROIRepositoriesResponse),
    (roi_calculator.ROISyncStatus, LiteLLMRoi.ROISyncStatus),
    (roi_calculator.ROIMetricsResponse, LiteLLMRoi.ROIMetricsResponse),
    (roi_calculator.ROIPersonResponse, LiteLLMRoi.ROIPersonResponse),
    (roi_calculator.ROIEstimateResponse, LiteLLMRoi.ROIEstimateResponse),
    (roi_calculator.ROIPullResponse, LiteLLMRoi.ROIPullResponse),
    (roi_calculator.ROITrendResponse, LiteLLMRoi.ROITrendResponse),
    (roi_calculator.ROISummaryResponse, LiteLLMRoi.ROISummaryResponse),
    (roi_calculator.ROIReportResponse, LiteLLMRoi.ROIReportResponse),
    (roi_calculator.ROIIdentityMapUpdate, LiteLLMRoi.ROIIdentityMapUpdate),
    (roi_calculator.ROIIdentityMapResponse, LiteLLMRoi.ROIIdentityMapResponse),
    (roi_calculator.ROIEstimatorChanges, LiteLLMRoi.ROIEstimatorChanges),
    (roi_calculator.ROIEstimatorFile, LiteLLMRoi.ROIEstimatorFile),
    (roi_calculator.ROIEstimatorCommit, LiteLLMRoi.ROIEstimatorCommit),
    (roi_calculator.ROIEstimatorEvidence, LiteLLMRoi.ROIEstimatorEvidence),
    (roi_calculator.ROICompletionRequest, LiteLLMRoi.ROICompletionRequest),
    (roi_calculator.ROICompletionResponse, LiteLLMRoi.ROICompletionResponse),
    (roi_calculator.ROIEstimatorResult, LiteLLMRoi.ROIEstimatorResult),
    (memory_management.MemoryDeleteResponse, LiteLLMMemory.MemoryDeleteResponse),
    (model_insights.ModelInsightMetric, LiteLLMInsights.ModelInsightMetric),
    (model_insights.ModelInsightDailyMetric, LiteLLMInsights.ModelInsightDailyMetric),
    (model_insights.ModelInsightDailyTotal, LiteLLMInsights.ModelInsightDailyTotal),
    (model_insights.ModelInsightTask, LiteLLMInsights.ModelInsightTask),
    (model_insights.ModelInsightTaskSummary, LiteLLMInsights.ModelInsightTaskSummary),
    (model_insights.ModelInsightsResponse, LiteLLMInsights.ModelInsightsResponse),
    (model_insights.ModelInsightTasksResponse, LiteLLMInsights.ModelInsightTasksResponse),
)

# docs/deviations/B7954.yaml: memory metadata is JsonValue and the row class is _MemoryRow
MEMORY_PAIRS: tuple[tuple[type[BaseModel], type[BaseModel], str], ...] = (
    (memory_management.LiteLLM_MemoryRow, LiteLLMMemory.LiteLLM_MemoryRow, "metadata"),
    (memory_management.MemoryCreateRequest, LiteLLMMemory.MemoryCreateRequest, "metadata"),
    (memory_management.MemoryUpdateRequest, LiteLLMMemory.MemoryUpdateRequest, "metadata"),
    (memory_management.MemoryListResponse, LiteLLMMemory.MemoryListResponse, "memories"),
)
CONFIG_PAIRS = (*MODEL_PAIRS, *[(ours, theirs) for ours, theirs, _ in MEMORY_PAIRS])

RECORD_PAIRS: tuple[tuple[SchemaSource, SchemaSource], ...] = (
    (TypeAdapter(roi_calculator.ROISpendRecord), TypeAdapter(LiteLLMRoi.ROISpendRecord)),
    (TypeAdapter(roi_calculator.ROIEstimate), TypeAdapter(LiteLLMRoi.ROIEstimate)),
    (TypeAdapter(roi_calculator.ROIPullRecord), TypeAdapter(LiteLLMRoi.ROIPullRecord)),
    (TypeAdapter(roi_calculator.ROIReport), TypeAdapter(LiteLLMRoi.ROIReport)),
    (TypeAdapter(roi_calculator.ROIPullFile), TypeAdapter(LiteLLMRoi.ROIPullFile)),
    (TypeAdapter(roi_calculator.ROIPullCommit), TypeAdapter(LiteLLMRoi.ROIPullCommit)),
    (TypeAdapter(roi_calculator.ROIPullEvidence), TypeAdapter(LiteLLMRoi.ROIPullEvidence)),
    (TypeAdapter(roi_calculator.ROIIdentityMatch), TypeAdapter(LiteLLMRoi.ROIIdentityMatch)),
    (TypeAdapter(roi_calculator.ROIPersonSummary), TypeAdapter(LiteLLMRoi.ROIPersonSummary)),
    (TypeAdapter(roi_calculator.ROIPullSummary), TypeAdapter(LiteLLMRoi.ROIPullSummary)),
    (TypeAdapter(roi_calculator.ROISummaryMetrics), TypeAdapter(LiteLLMRoi.ROISummaryMetrics)),
    (TypeAdapter(roi_calculator.ROITrendDay), TypeAdapter(LiteLLMRoi.ROITrendDay)),
    (TypeAdapter(roi_calculator.ROISummary), TypeAdapter(LiteLLMRoi.ROISummary)),
    (
        TypeAdapter(roi_calculator.ROICompletionMessage),
        TypeAdapter(LiteLLMRoi.ROICompletionMessage),
    ),
    (
        TypeAdapter(roi_calculator.ROICompletionMetadata),
        TypeAdapter(LiteLLMRoi.ROICompletionMetadata),
    ),
    (TypeAdapter(roi_calculator.ROIResponseFormat), TypeAdapter(LiteLLMRoi.ROIResponseFormat)),
    (
        TypeAdapter(model_insights.ModelInsightsMetric),
        TypeAdapter(LiteLLMInsights.ModelInsightsMetric),
    ),
)

ESTIMATE_PAYLOAD: dict[str, JsonValue] = {"status": "error", "hours": None, "reasoning": "r"}
METRICS_PAYLOAD: dict[str, JsonValue] = {
    "matched_spend": 1.5,
    "output_hours": 2.5,
    "total_spend": 3.5,
    "total_output_hours": 4.5,
    "excluded_spend": 0.5,
    "cost_per_hour": None,
    "hours_per_dollar": None,
    "merged_prs": 1,
    "estimated_prs": 2,
    "matched_prs": 3,
    "cohort_people": 4,
    "people_with_prs": 5,
    "pending_prs": 6,
}
PULL_PAYLOAD: dict[str, JsonValue] = {
    "repo": "a/b",
    "number": 1,
    "title": "t",
    "url": "u",
    "login": "l",
    "emails": [],
    "profile_email": "",
    "merged_at": "2026-01-01",
    "head_sha": "s",
    "additions": 1,
    "deletions": 2,
    "changed_files": 3,
    "commit_count": 4,
    "incomplete_metadata": False,
    "estimate": ESTIMATE_PAYLOAD,
    "email": "e",
    "match_method": "manual",
    "matched": True,
}
SYNC_PAYLOAD: dict[str, JsonValue] = {
    "running": False,
    "phase": "idle",
    "stage": "s",
    "done": 0,
    "total": 0,
    "estimated": 0,
    "reused": 0,
    "needs_attention": 0,
    "error": None,
}
COMPLETION_PAYLOAD: dict[str, JsonValue] = {
    "model": "m",
    "temperature": 0,
    "messages": [{"role": "user", "content": "c"}],
    "response_format": {"type": "json_object"},
    "max_tokens": 1200,
    "metadata": {"tags": ["t"], "litellm_roi_estimator": True},
}
INSIGHT_METRIC_PAYLOAD: dict[str, JsonValue] = {
    "model_group": "g",
    "model": "m",
    "provider": "p",
    "spend": 1.5,
    "prompt_tokens": 1,
    "completion_tokens": 2,
    "requests": 3,
    "successful_requests": 2,
    "failed_requests": 1,
}
INSIGHT_TOTAL_PAYLOAD: dict[str, JsonValue] = {
    "date": "2026-01-01",
    "spend": 1.5,
    "prompt_tokens": 1,
    "completion_tokens": 2,
    "requests": 3,
}
TASK_SUMMARY_PAYLOAD: dict[str, JsonValue] = {
    "task_type": "t",
    "label": "l",
    "category": "c",
    "value": 1.5,
    "share": 0.5,
    "leader": "l",
    "provider": "p",
}
MEMORY_ROW_PAYLOAD: dict[str, JsonValue] = {"memory_id": "m", "key": "k", "value": "v"}

PROBES: tuple[tuple[type[BaseModel], type[BaseModel], tuple[JsonValue, ...]], ...] = (
    (
        roi_calculator.ROIIdentityMapUpdate,
        LiteLLMRoi.ROIIdentityMapUpdate,
        (
            {"github_login": "  MixedCase ", "email": None},
            {"github_login": " User ", "email": "u@example.com"},
            {"github_login": "user_name", "email": None},
            {"github_login": "user[name]", "email": None},
            {"github_login": "a-b", "email": None},
            {"github_login": "-", "email": None},
            {"github_login": "bad name", "email": None},
            {"github_login": "", "email": None},
            {"github_login": "   ", "email": None},
            {"github_login": "Üser", "email": None},
            {"github_login": "user.name", "email": None},
            {"github_login": "user"},
        ),
    ),
    (
        roi_calculator.ROISettings,
        LiteLLMRoi.ROISettings,
        (
            {},
            {"github_api_url": " https://example.com/ "},
            {"github_api_url": "http://example.com"},
            {"github_api_url": "https://user:pass@example.com"},
            {"github_api_url": "https://:pass@example.com"},
            {"github_api_url": "https://user@example.com"},
            {"github_api_url": "https://example.com?q=1"},
            {"github_api_url": "https://example.com#f"},
            {"github_api_url": ""},
            {"github_api_url": "https://"},
            {"github_token": "abc", "estimator_key": "def"},
            {"repos": ["Owner/Repo.git", "owner/repo/", "owner/repo"]},
            {"repos": ["bad repo"]},
            {"repos": ["a/../b"]},
            {"repos": ["a/b", "."]},
            {"repos": ["./b"]},
            {"repos": ["a/.."]},
            {"repos": [" owner/repo "]},
            {"estimator_prompt": "  hi  "},
            {"estimator_prompt": "   "},
            {"estimator_prompt": "x" * 20000},
            {"estimator_prompt": "x" * 20001},
            {"backfill_days": 1},
            {"backfill_days": 3650},
            {"backfill_days": 0},
            {"backfill_days": 3651},
            {"update_interval_minutes": 0},
            {"update_interval_minutes": 4.9},
            {"update_interval_minutes": 5},
            {"update_interval_minutes": 43200},
            {"update_interval_minutes": 43200.5},
            {"update_interval_minutes": -1},
            {"update_interval_minutes": math.inf},
            {"update_interval_minutes": math.nan},
            {"identity_map": {"User": "User@Example.COM"}},
            {"identity_map": {"user": "user@users.noreply.github.com"}},
            {"identity_map": {"user": " user@example.com "}},
            {"identity_map": {"user": "not-an-email"}},
            {"identity_map": {"bad login": "user@example.com"}},
            {"identity_map": {"User": "a@example.com", "user": "b@example.com"}},
            {"unknown_key": 1},
        ),
    ),
    (
        roi_calculator.ROISettingsUpdate,
        LiteLLMRoi.ROISettingsUpdate,
        (
            {},
            {"unknown_key": 1},
            {"repos": ["a/b"]},
            {"backfill_days": 1},
            {"backfill_days": 3650},
            {"backfill_days": 0},
            {"backfill_days": 3651},
            {"update_interval_minutes": 0},
            {"update_interval_minutes": 4},
            {"update_interval_minutes": 43200},
            {"update_interval_minutes": -1},
            {"update_interval_minutes": 43201},
            {"update_interval_minutes": math.inf},
            {"update_interval_minutes": math.nan},
        ),
    ),
    (
        roi_calculator.ROISettingsResponse,
        LiteLLMRoi.ROISettingsResponse,
        (
            {
                "github_api_url": "https://api.github.com",
                "repos": [],
                "estimator_model": "m",
                "estimator_prompt": "p",
                "backfill_days": 7,
                "update_interval_minutes": 1440,
                "has_estimator_key": False,
                "identity_map": {},
                "has_github_token": False,
                "default_prompt": "d",
                "available_models": [],
                "ready": True,
            },
        ),
    ),
    (
        roi_calculator.ROIRepositoriesResponse,
        LiteLLMRoi.ROIRepositoriesResponse,
        (
            {
                "repositories": [{"name": "a/b", "visibility": "private", "archived": False}],
                "page": 1,
                "has_more": False,
            },
        ),
    ),
    (
        roi_calculator.ROISyncStatus,
        LiteLLMRoi.ROISyncStatus,
        ({**SYNC_PAYLOAD}, {**SYNC_PAYLOAD, "phase": "bogus"}),
    ),
    (roi_calculator.ROIMetricsResponse, LiteLLMRoi.ROIMetricsResponse, (METRICS_PAYLOAD,)),
    (
        roi_calculator.ROIPersonResponse,
        LiteLLMRoi.ROIPersonResponse,
        (
            {
                "id": "u",
                "email": "e",
                "logins": ["l"],
                "spend": None,
                "hours": 1.0,
                "prs": 2,
                "estimated_prs": 3,
                "pending_prs": 4,
                "match_methods": ["manual"],
                "eligible": True,
                "cost_per_hour": None,
            },
        ),
    ),
    (
        roi_calculator.ROIEstimateResponse,
        LiteLLMRoi.ROIEstimateResponse,
        ({"status": "estimated", "hours": 1.0, "reasoning": "because"},),
    ),
    (
        roi_calculator.ROIPullResponse,
        LiteLLMRoi.ROIPullResponse,
        (
            PULL_PAYLOAD,
            {**PULL_PAYLOAD, "estimate": {"status": "bogus", "hours": None, "reasoning": "r"}},
        ),
    ),
    (
        roi_calculator.ROISummaryResponse,
        LiteLLMRoi.ROISummaryResponse,
        (
            {
                "id": None,
                "mode": "m",
                "start": "s",
                "end": "e",
                "synced_at": "t",
                "repos": [],
                "estimator_model": "m",
                "estimator_prompt": "p",
                "warnings": [],
                "effort_basis": None,
                "metrics": METRICS_PAYLOAD,
                "people": [],
                "pulls": [],
                "trend": [{"date": "d", "spend": 1.0, "hours": 2.0, "prs": 3}],
            },
        ),
    ),
    (roi_calculator.ROIReportResponse, LiteLLMRoi.ROIReportResponse, ({"report": None},)),
    (
        roi_calculator.ROIIdentityMapResponse,
        LiteLLMRoi.ROIIdentityMapResponse,
        ({"report": None, "identity_map": {}},),
    ),
    (
        roi_calculator.ROIEstimatorChanges,
        LiteLLMRoi.ROIEstimatorChanges,
        ({"additions": 1, "deletions": 2, "files": 3, "commits": 4},),
    ),
    (
        roi_calculator.ROIEstimatorFile,
        LiteLLMRoi.ROIEstimatorFile,
        ({"filename": None, "status": None, "additions": None, "deletions": None},),
    ),
    (
        roi_calculator.ROIEstimatorCommit,
        LiteLLMRoi.ROIEstimatorCommit,
        ({"sha": "s", "message": "m"},),
    ),
    (
        roi_calculator.ROIEstimatorEvidence,
        LiteLLMRoi.ROIEstimatorEvidence,
        (
            {
                "repo": "a/b",
                "number": 1,
                "title": "t",
                "body": "b",
                "changes": {"additions": 1, "deletions": 2, "files": 3, "commits": 4},
                "files": [{"filename": "f", "status": "modified", "additions": 1, "deletions": 2}],
                "commits": [{"sha": "s", "message": "m"}],
            },
        ),
    ),
    (
        roi_calculator.ROICompletionRequest,
        LiteLLMRoi.ROICompletionRequest,
        (
            COMPLETION_PAYLOAD,
            {**COMPLETION_PAYLOAD, "temperature": 1},
            {**COMPLETION_PAYLOAD, "max_tokens": 1},
            {**COMPLETION_PAYLOAD, "reasoning_effort": "none"},
            {**COMPLETION_PAYLOAD, "reasoning_effort": "low"},
            {**COMPLETION_PAYLOAD, "messages": [{"role": "assistant", "content": "c"}]},
        ),
    ),
    (
        roi_calculator.ROICompletionResponse,
        LiteLLMRoi.ROICompletionResponse,
        (
            {"choices": [{"finish_reason": "stop", "message": {"content": "x"}}]},
            {"choices": [{"message": {}}]},
        ),
    ),
    (
        roi_calculator.ROIEstimatorResult,
        LiteLLMRoi.ROIEstimatorResult,
        (
            {"hours": 1, "reasoning": "x"},
            {"hours": 1.5, "reasoning": "x"},
            {"hours": 0, "reasoning": "x"},
            {"hours": -1, "reasoning": "x"},
            {"hours": math.inf, "reasoning": "x"},
            {"hours": math.nan, "reasoning": "x"},
            {"hours": True, "reasoning": "x"},
            {"hours": "1", "reasoning": "x"},
            {"hours": 1, "reasoning": "  "},
            {"hours": 1, "reasoning": "x", "extra": 1},
            {"reasoning": "x"},
        ),
    ),
    (
        memory_management.LiteLLM_MemoryRow,
        LiteLLMMemory.LiteLLM_MemoryRow,
        (
            {},
            MEMORY_ROW_PAYLOAD,
            {
                **MEMORY_ROW_PAYLOAD,
                "metadata": {"a": [1, True, None]},
                "user_id": "u",
                "team_id": "t",
                "created_at": "2026-01-02T03:04:05Z",
                "created_by": "c",
                "updated_at": "2026-01-02T03:04:05Z",
                "updated_by": "d",
            },
            {**MEMORY_ROW_PAYLOAD, "metadata": 3},
            {**MEMORY_ROW_PAYLOAD, "metadata": [1]},
            {**MEMORY_ROW_PAYLOAD, "metadata": "s"},
            {"memory_id": "m"},
        ),
    ),
    (
        memory_management.MemoryCreateRequest,
        LiteLLMMemory.MemoryCreateRequest,
        (
            {"key": "k", "value": "v"},
            {"key": "k", "value": "v", "metadata": {"x": 1}, "user_id": "u", "team_id": "t"},
            {"value": "v"},
        ),
    ),
    (
        memory_management.MemoryUpdateRequest,
        LiteLLMMemory.MemoryUpdateRequest,
        ({}, {"value": "v", "metadata": [1, 2], "user_id": "u", "team_id": "t"}),
    ),
    (
        memory_management.MemoryListResponse,
        LiteLLMMemory.MemoryListResponse,
        ({"memories": [], "total": 0}, {"memories": [MEMORY_ROW_PAYLOAD], "total": 1}),
    ),
    (
        memory_management.MemoryDeleteResponse,
        LiteLLMMemory.MemoryDeleteResponse,
        ({"key": "k", "deleted": True}, {"key": "k"}),
    ),
    (
        model_insights.ModelInsightMetric,
        LiteLLMInsights.ModelInsightMetric,
        (INSIGHT_METRIC_PAYLOAD, {"model_group": "g"}),
    ),
    (
        model_insights.ModelInsightDailyMetric,
        LiteLLMInsights.ModelInsightDailyMetric,
        ({**INSIGHT_METRIC_PAYLOAD, "date": "2026-01-01"}, INSIGHT_METRIC_PAYLOAD),
    ),
    (
        model_insights.ModelInsightDailyTotal,
        LiteLLMInsights.ModelInsightDailyTotal,
        (INSIGHT_TOTAL_PAYLOAD,),
    ),
    (
        model_insights.ModelInsightTask,
        LiteLLMInsights.ModelInsightTask,
        ({"task_type": "t", "label": "l", "category": "c"},),
    ),
    (
        model_insights.ModelInsightTaskSummary,
        LiteLLMInsights.ModelInsightTaskSummary,
        (
            TASK_SUMMARY_PAYLOAD,
            {"task_type": "t", "label": "l", "category": "c", "value": 1.5, "share": 0.5},
        ),
    ),
    (
        model_insights.ModelInsightsResponse,
        LiteLLMInsights.ModelInsightsResponse,
        (
            {
                "start_date": "2026-01-01",
                "end_date": "2026-01-02",
                "daily": [{**INSIGHT_METRIC_PAYLOAD, "date": "2026-01-01"}],
                "daily_totals": [INSIGHT_TOTAL_PAYLOAD],
                "top_models": [INSIGHT_METRIC_PAYLOAD],
            },
        ),
    ),
    (
        model_insights.ModelInsightTasksResponse,
        LiteLLMInsights.ModelInsightTasksResponse,
        ({"start_date": "2026-01-01", "end_date": "2026-01-02", "tasks": [TASK_SUMMARY_PAYLOAD]},),
    ),
)
CASES = [(ours, theirs, payload) for ours, theirs, payloads in PROBES for payload in payloads]


@pytest.mark.parametrize(("ours", "theirs"), MODEL_PAIRS)
def test_model_names_match(ours: type[BaseModel], theirs: type[BaseModel]) -> None:
    our_schema: dict[str, JsonValue] = ours.model_json_schema()
    their_schema: dict[str, JsonValue] = theirs.model_json_schema()
    assert our_schema == their_schema


@pytest.mark.parametrize(("ours", "theirs", "field"), MEMORY_PAIRS)
def test_memory_names_match(ours: type[BaseModel], theirs: type[BaseModel], field: str) -> None:
    our_raw: dict[str, JsonValue] = ours.model_json_schema()
    their_raw: dict[str, JsonValue] = theirs.model_json_schema()
    adapter = TypeAdapter(ObjectSchema)
    our_schema = adapter.validate_python(our_raw)
    their_schema = adapter.validate_python(their_raw)
    field_adapter = TypeAdapter(FieldSchema)
    our_field = field_adapter.validate_python(our_schema["properties"].pop(field))
    their_field = field_adapter.validate_python(their_schema["properties"].pop(field))
    assert our_schema == their_schema
    assert our_field == their_field


@pytest.mark.parametrize(("ours", "theirs"), CONFIG_PAIRS)
def test_config_names_match(ours: type[BaseModel], theirs: type[BaseModel]) -> None:
    our_bases = [base.__name__ for base in ours.__bases__]
    assert our_bases == [base.__name__ for base in theirs.__bases__]
    assert ours.model_config.get("extra") == theirs.model_config.get("extra")
    assert ours.model_config.get("frozen") == theirs.model_config.get("frozen")
    assert ours.model_config.get("strict") == theirs.model_config.get("strict")
    assert ours.model_config.get("from_attributes") == theirs.model_config.get("from_attributes")


@pytest.mark.parametrize(("ours", "theirs"), RECORD_PAIRS)
def test_record_names_match(ours: SchemaSource, theirs: SchemaSource) -> None:
    assert ours.json_schema() == theirs.json_schema()


def test_constant_names_match() -> None:
    assert roi_calculator.DEFAULT_PROMPT == LiteLLMRoi.DEFAULT_PROMPT


@pytest.mark.parametrize("payload", [{}, {"identity_map": {"User": "u@example.com"}}])
def test_settings_repr_names_match(payload: dict[str, JsonValue]) -> None:
    # the repr shows that identity_map stays a read-only mappingproxy, as in LiteLLM
    ours = roi_calculator.ROISettings.model_validate(payload)
    theirs = LiteLLMRoi.ROISettings.model_validate(payload)
    assert repr(ours) == repr(theirs)


@pytest.mark.parametrize(("ours", "theirs", "payload"), CASES)
def test_behavior_names_match(
    ours: type[BaseModel], theirs: type[BaseModel], payload: JsonValue,
) -> None:
    try:
        # ROISettings keeps identity_map as a mappingproxy, which pydantic doesn't write as JSON
        expected: dict[str, JsonValue] = theirs.model_validate(payload).model_dump(
            mode="json", fallback=dict[str, str],
        )
    except ValidationError as error:
        with pytest.raises(ValidationError) as raised:
            ours.model_validate(payload)
        our_errors: Sequence[ErrorShape] = raised.value.errors(include_url=False)
        their_errors: Sequence[ErrorShape] = error.errors(include_url=False)
        assert [(item["type"], item["loc"], item["msg"]) for item in our_errors] == [
            (item["type"], item["loc"], item["msg"]) for item in their_errors
        ]
        return
    validated: dict[str, JsonValue] = ours.model_validate(payload).model_dump(
        mode="json", fallback=dict[str, str],
    )
    assert validated == expected


def test_attribute_names_match() -> None:
    message = CompletionMessage(content="x")
    completion = Completion(choices=(CompletionChoice(finish_reason="stop", message=message),))
    response = roi_calculator.ROICompletionResponse.model_validate(completion)
    ours: dict[str, JsonValue] = response.model_dump(mode="json")
    reference = LiteLLMRoi.ROICompletionResponse.model_validate(completion)
    theirs: dict[str, JsonValue] = reference.model_dump(mode="json")
    assert ours == theirs
    assert ours == {"choices": [{"finish_reason": "stop", "message": {"content": "x"}}]}
