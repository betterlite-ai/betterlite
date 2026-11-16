from datetime import datetime
from typing import Literal, TypedDict

from pydantic import BaseModel, ConfigDict, Field, JsonValue


class JobRecord(BaseModel):
    id: str
    target_type: str
    target_id: str
    stopped_at: datetime | None = None
    router_name: str
    router_names: tuple[str, ...] = ()
    models: frozenset[str] = frozenset()
    direction: str = "forward"
    baseline_model: str | None = None
    shadow_percentage: float
    judge_model: str
    max_turns: int
    max_budget: float | None = None
    ends_at: datetime


class JobStat(TypedDict):
    job_id: str
    attempts: int
    spend: str


class EndsAfter(TypedDict):
    gt: datetime


class JobsWhere(TypedDict):
    stopped_at: None
    ends_at: EndsAfter


class AttemptData(TypedDict):
    job_id: str
    request_id: str
    router_name: str | None
    outcome: str
    tier: str | None
    real_model: str | None
    shadow_model: str | None
    confidence: float | None
    judge_cost: float
    shadow_cost: float
    shadow_classifier_cost: float
    real_cost: float
    real_classifier_cost: float
    real_cache_hit: bool
    error: str | None


class Price(TypedDict):
    input_cost_per_token: float
    output_cost_per_token: float


class JobIdFilter(BaseModel):
    in_ids: list[str] = Field(alias="in")


class GroupByWhere(BaseModel):
    job_id: JobIdFilter


class GroupByArgs(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    by: list[str]
    count: bool
    sum_costs: dict[str, bool] = Field(alias="sum")
    where: GroupByWhere


class Input(TypedDict):
    operation: Literal["validate_job", "load_jobs", "record_attempt"]
    arm_router: str
    rows: list[dict[str, JsonValue]]
    stats: list[JobStat]
    metadata: dict[str, JsonValue]
    model_group: str
    aliases: dict[str, str]
    hook_calls: int
    job: dict[str, JsonValue]
    row: dict[str, JsonValue]
    request_id: str
    real_text: str
    shadow_reply: str
    judge_reply: str
    shadow_price: Price
    judge_price: Price
    real_cost: float
    shadow_error: str | None
