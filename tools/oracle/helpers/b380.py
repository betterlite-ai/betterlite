from datetime import datetime
from typing import Literal, TypedDict

import litellm
from litellm.types.management_endpoints.auto_router_endpoints import (
    ShadowEvalJobResponse,
    ShadowEvalJobTargetResponse,
)
from pydantic import TypeAdapter


class TargetInput(TypedDict):
    target_type: Literal["key", "team", "user"]
    target_id: str
    max_turns: int
    max_budget: float | None
    stopped_at: str | None
    attempt_count: int | None
    spend: float | None
    target_alias: str | None
    key_name: str | None


class JobInput(TypedDict):
    job_id: str
    targets: list[TargetInput]
    router_names: list[str]
    models: list[str]
    direction: Literal["forward", "reverse"]
    baseline_model: str | None
    judge_model: str
    shadow_percentage: float
    created_at: str
    ends_at: str
    stopped_by: str | None


class Input(TypedDict):
    job: JobInput


class TargetView(TypedDict):
    target_type: Literal["key", "team", "user"]
    target_id: str
    max_turns: int
    max_budget: float | None
    stopped_at: str | None
    attempt_count: int | None
    spend: float | None
    verdicts: None
    target_alias: str | None
    key_name: str | None


class ResponseView(TypedDict):
    job_id: str
    targets: list[TargetView]
    router_names: list[str]
    models: list[str]
    direction: Literal["forward", "reverse"]
    baseline_model: str | None
    judge_model: str
    shadow_percentage: float
    created_at: str
    ends_at: str
    stopped_by: str | None
    judged_count: int | None
    error_count: int | None
    judge_spend: float | None
    last_error: str | None
    results: None
    router_name: str
    status: Literal["running", "completed", "stopped"]


class Output(TypedDict):
    response: ResponseView
    budget_spent: list[bool]


DATETIME: TypeAdapter[datetime] = TypeAdapter(datetime)
RESPONSE: TypeAdapter[ResponseView] = TypeAdapter(ResponseView)


def target(item: TargetInput) -> ShadowEvalJobTargetResponse:
    stopped_at = item["stopped_at"]
    return ShadowEvalJobTargetResponse(
        target_type=item["target_type"],
        target_id=item["target_id"],
        max_turns=item["max_turns"],
        max_budget=item["max_budget"],
        stopped_at=None if stopped_at is None else DATETIME.validate_python(stopped_at),
        attempt_count=item["attempt_count"],
        spend=item["spend"],
        target_alias=item["target_alias"],
        key_name=item["key_name"],
    )


def run(input_value: Input) -> Output:
    litellm.suppress_debug_info = True
    job_input = input_value["job"]
    response = ShadowEvalJobResponse(
        job_id=job_input["job_id"],
        targets=tuple(target(item) for item in job_input["targets"]),
        router_names=tuple(job_input["router_names"]),
        models=tuple(job_input["models"]),
        direction=job_input["direction"],
        baseline_model=job_input["baseline_model"],
        judge_model=job_input["judge_model"],
        shadow_percentage=job_input["shadow_percentage"],
        created_at=DATETIME.validate_python(job_input["created_at"]),
        ends_at=DATETIME.validate_python(job_input["ends_at"]),
        stopped_by=job_input["stopped_by"],
    )
    return Output(
        response=RESPONSE.validate_json(response.model_dump_json()),
        budget_spent=[item.budget_spent for item in response.targets],
    )
