from datetime import datetime
from typing import Literal

from litellm.types.management_endpoints.auto_router_endpoints import (
    ShadowEvalJobResponse,
    ShadowEvalJobTargetResponse,
)
from pydantic import TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

DATETIME: TypeAdapter[datetime] = TypeAdapter(datetime)


class Leg(Strict):
    target_type: Literal["key", "team", "user"]
    target_id: str
    max_turns: int
    max_budget: float | None = None
    attempt_count: int | None = None
    stopped_at: str | None = None
    spend: float | None = None


class Job(Strict):
    job_id: str
    router_names: list[str]
    ends_at: str
    stopped_by: str | None = None
    targets: list[Leg]


class Response(Strict):
    job_id: str
    router_name: str
    status: Literal["running", "completed", "stopped"]
    budget_spent: list[bool]


class Output(Strict):
    responses: list[Response]


def target(leg: Leg) -> ShadowEvalJobTargetResponse:
    return ShadowEvalJobTargetResponse(
        target_type=leg.target_type,
        target_id=leg.target_id,
        max_turns=leg.max_turns,
        max_budget=leg.max_budget,
        stopped_at=None if leg.stopped_at is None else DATETIME.validate_python(leg.stopped_at),
        attempt_count=leg.attempt_count,
        spend=leg.spend,
    )


def response(job: Job) -> Response:
    ends_at = DATETIME.validate_python(job.ends_at)
    evaluated = ShadowEvalJobResponse(
        job_id=job.job_id,
        targets=tuple(target(leg) for leg in job.targets),
        router_names=tuple(job.router_names),
        judge_model="",
        shadow_percentage=0.0,
        created_at=ends_at,
        ends_at=ends_at,
        stopped_by=job.stopped_by,
    )
    return Response(
        job_id=evaluated.job_id,
        router_name=evaluated.router_name,
        status=evaluated.status,
        budget_spent=[item.budget_spent for item in evaluated.targets],
    )


class ShadowEvalModels(Operation):
    jobs: list[Job]

    @override
    def run(self) -> Output:
        return Output(responses=[response(job) for job in self.jobs])
