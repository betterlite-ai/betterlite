import asyncio
import os
import sys
from collections.abc import Mapping, Sequence
from types import ModuleType
from typing import Final, Literal

import litellm
from fastapi import HTTPException
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.management_endpoints.auto_router_endpoints import (
    list_shadow_eval_jobs,
    start_shadow_eval,
)
from litellm.router import Router
from litellm.types.management_endpoints.auto_router_endpoints import StartShadowEvalRequest
from pydantic import Field, JsonValue, ValidationError
from typing_extensions import override

from probes.protocol import MAX_MESSAGE_BYTES, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
FORBIDDEN: Final = 403
BAD_REQUEST: Final = 400
Role = Literal["absent", "proxy_admin", "proxy_admin_viewer", "internal_user", "customer"]


class Deployment(Strict):
    model_name: str
    litellm_params: dict[str, JsonValue]


class KeyRow(Strict):
    token: str
    team_id: str | None = None


class TeamRow(Strict):
    team_id: str


class UserRow(Strict):
    user_id: str


class Targets(Strict):
    keys: list[KeyRow] = Field(default_factory=list)
    teams: list[str] = Field(default_factory=list)
    users: list[str] = Field(default_factory=list)


class Body(Strict):
    text: str
    pad_to: int | None = None


class Step(Strict):
    parse: Body | None = None
    access: Role | None = None
    validate_start: Body | None = None


class RequestView(Strict):
    api_key_ids: list[str]
    team_ids: list[str]
    user_ids: list[str]
    models: list[str]
    router_names: list[str]
    direction: Literal["forward", "reverse"]
    baseline_model: str | None
    shadow_percentage: str
    judge_model: str
    duration_days: int
    max_budget: str


class ParseAccepted(Strict):
    outcome: Literal["accepted"] = "accepted"
    request: RequestView


class ParseRejected(Strict):
    outcome: Literal["rejected"] = "rejected"


class ParseOverLimit(Strict):
    outcome: Literal["over_limit"] = "over_limit"
    bytes: int
    limit: int


class AccessResult(Strict):
    writer: bool
    viewer: bool


class Validated(Strict):
    outcome: Literal["validated"] = "validated"


class StartRejected(Strict):
    outcome: Literal["rejected"] = "rejected"
    status: int


class StepOutput(Strict):
    step: Literal["parse", "access", "validate_start"]
    result: ParseAccepted | ParseRejected | ParseOverLimit | AccessResult | Validated | StartRejected


class Output(Strict):
    steps: list[StepOutput]


class Table:
    def __init__(self, rows: Sequence[KeyRow | TeamRow | UserRow]) -> None:
        self.rows = rows

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[KeyRow | TeamRow | UserRow]:
        _ = where
        return self.rows


class Sentinel(Exception):
    pass


class FakeDb:
    litellm_verificationtoken: Table
    litellm_teamtable: Table
    litellm_usertable: Table

    def __init__(self, rows: Targets) -> None:
        self.litellm_verificationtoken = Table(rows.keys)
        self.litellm_teamtable = Table([TeamRow(team_id=team) for team in rows.teams])
        self.litellm_usertable = Table([UserRow(user_id=user) for user in rows.users])

    async def query_raw(self, _query: str, *_args: object) -> Sequence[Mapping[str, object]]:
        raise Sentinel

    async def execute_raw(self, _query: str, *_args: object) -> int:
        raise Sentinel


class FakePrisma:
    db: FakeDb

    def __init__(self, rows: Targets) -> None:
        self.db = FakeDb(rows)


class ProxyModule(ModuleType):
    llm_router: Router | None
    prisma_client: FakePrisma


def install_proxy(router: Router | None, rows: Targets) -> None:
    module = ProxyModule(PROXY_MODULE)
    module.llm_router = router
    module.prisma_client = FakePrisma(rows)
    sys.modules[PROXY_MODULE] = module


def unconfigured() -> None:
    litellm.api_key = None
    litellm.anthropic_key = None
    _ = os.environ.pop("ANTHROPIC_API_KEY", None)
    _ = os.environ.pop("ANTHROPIC_AUTH_TOKEN", None)


def router_of(deployments: list[Deployment]) -> Router:
    model_list: list[dict[str, JsonValue]] = [
        {"model_name": deployment.model_name, "litellm_params": deployment.litellm_params}
        for deployment in deployments
    ]
    return Router(model_list=model_list)


def caller(role: Role) -> UserAPIKeyAuth:
    if role == "absent":
        return UserAPIKeyAuth()
    return UserAPIKeyAuth(user_role=LitellmUserRoles(role))


def body_bytes(body: Body) -> bytes:
    encoded = body.text.encode()
    total = len(encoded) if body.pad_to is None else body.pad_to
    limit = MAX_MESSAGE_BYTES + 1
    if total < len(encoded) or total > limit:
        raise ProbeError("invalid_request", f"a padded body must be between {len(encoded)} and {limit} bytes")
    return encoded + b" " * (total - len(encoded))


def request_view(request: StartShadowEvalRequest) -> RequestView:
    return RequestView(
        api_key_ids=list(request.api_key_ids),
        team_ids=list(request.team_ids),
        user_ids=list(request.user_ids),
        models=list(request.models),
        router_names=list(request.router_names),
        direction=request.direction,
        baseline_model=request.baseline_model,
        shadow_percentage=f"{request.shadow_percentage:.2f}",
        judge_model=request.judge_model,
        duration_days=request.duration_days,
        max_budget=f"{request.max_budget:.15f}",
    )


def parse(body: Body) -> ParseAccepted | ParseRejected:
    try:
        request = StartShadowEvalRequest.model_validate_json(body_bytes(body))
    except ValidationError:
        return ParseRejected()
    return ParseAccepted(request=request_view(request))


async def call(
    request: StartShadowEvalRequest | None,
    auth: UserAPIKeyAuth,
    router: Router | None,
    rows: Targets,
) -> None:
    install_proxy(router, rows)
    if request is None:
        _ = await list_shadow_eval_jobs(auth)
    else:
        _ = await start_shadow_eval(request, auth)


def verdict(auth: UserAPIKeyAuth, *, writer: bool) -> bool:
    action = "start a shadow eval" if writer else "view shadow evals"
    request = (
        StartShadowEvalRequest(
            api_key_ids=("probe-key",), router_names=("probe-router",), shadow_percentage=50.0
        )
        if writer
        else None
    )
    try:
        asyncio.run(call(request, auth, Router(model_list=[]) if writer else None, Targets()))
    except HTTPException as error:
        if error.status_code == FORBIDDEN:
            return False
        if writer and error.status_code == BAD_REQUEST:
            return True
        raise ProbeError("invalid_request", f"{action} answered {error.status_code}") from error
    except Sentinel:
        return True
    return True


def access(role: Role) -> AccessResult:
    auth = caller(role)
    return AccessResult(writer=verdict(auth, writer=True), viewer=verdict(auth, writer=False))


def start_result(body: Body, router: Router, rows: Targets) -> Validated | StartRejected:
    request = StartShadowEvalRequest.model_validate_json(body_bytes(body))
    auth = UserAPIKeyAuth(user_role=LitellmUserRoles.PROXY_ADMIN)
    try:
        asyncio.run(call(request, auth, router, rows))
    except HTTPException as error:
        return StartRejected(status=error.status_code)
    except Sentinel:
        return Validated()
    return Validated()


class ShadowEvalValidation(Operation):
    router: list[Deployment] = Field(default_factory=list)
    rows: Targets = Field(default_factory=Targets)
    steps: list[Step]

    @override
    def run(self) -> Strict:
        router = router_of(self.router)
        steps: list[StepOutput] = []
        unconfigured()
        for step in self.steps:
            if step.access is not None:
                steps.append(StepOutput(step="access", result=access(step.access)))
            elif step.validate_start is not None:
                steps.append(
                    StepOutput(
                        step="validate_start",
                        result=start_result(step.validate_start, router, self.rows),
                    )
                )
            elif step.parse is not None:
                steps.append(StepOutput(step="parse", result=parse(step.parse)))
            else:
                raise ProbeError("invalid_request", "a step names no action")
        return Output(steps=steps)
