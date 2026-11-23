import asyncio
import os
import sys
from collections.abc import Mapping, Sequence
from datetime import datetime, timezone, tzinfo
from types import ModuleType
from typing import Final, Literal
from uuid import UUID

import litellm
from fastapi import HTTPException
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.management_endpoints import auto_router_endpoints as endpoints
from litellm.proxy.management_endpoints.auto_router_endpoints import (
    start_shadow_eval,
    stop_shadow_eval_job,
)
from litellm.router import Router
from litellm.types.management_endpoints import auto_router_endpoints as response_types
from litellm.types.management_endpoints.auto_router_endpoints import StartShadowEvalRequest
from litellm.types.router import RouterModelGroupAliasItem
from prisma.errors import UniqueViolationError
from pydantic import Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import MAX_MESSAGE_BYTES, Operation, ProbeError, Strict

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
BAD_REQUEST: Final = 400
Role = Literal["absent", "proxy_admin", "proxy_admin_viewer", "internal_user", "customer"]
Direction = Literal["forward", "reverse"]
TargetType = Literal["key", "team", "user"]
StatusText = Literal["running", "completed", "stopped"]

_NOW: datetime = datetime(2023, 11, 14, tzinfo=timezone.utc)
_MINTED: list[UUID] = []


class FixedInstant(datetime):
    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> "FixedInstant":
        _ = tz
        return cls.fromtimestamp(_NOW.timestamp(), timezone.utc)


def _mint() -> UUID:
    if not _MINTED:
        raise ProbeError("invalid_request", "the case ran out of minted ids")
    return _MINTED.pop(0)


def _instant(millis: int) -> datetime:
    return datetime.fromtimestamp(millis / 1000, timezone.utc)


def _millis(stamp: datetime) -> int:
    aware = stamp if stamp.tzinfo is not None else stamp.replace(tzinfo=timezone.utc)
    return round(aware.timestamp() * 1000)


class Deployment(Strict):
    model_name: str
    litellm_params: dict[str, JsonValue]


class KeySeed(Strict):
    token: str
    key_alias: str | None = None
    key_name: str | None = None
    team_id: str | None = None


class TeamSeed(Strict):
    team_id: str
    team_alias: str | None = None


class UserSeed(Strict):
    user_id: str
    user_email: str | None = None


class Rows(Strict):
    keys: list[KeySeed] = Field(default_factory=list)
    teams: list[TeamSeed] = Field(default_factory=list)
    users: list[UserSeed] = Field(default_factory=list)


class JobSeed(Strict):
    id: str
    group_id: str
    target_type: TargetType
    target_id: str
    router_name: str
    router_names: list[str] = Field(default_factory=list)
    models: list[str] = Field(default_factory=list)
    direction: Direction
    baseline_model: str | None = None
    judge_model: str
    shadow_percentage: str
    max_turns: int
    max_budget: str | None
    created_at_millis: int
    ends_at_millis: int
    stopped_at_millis: int | None = None
    stopped_by: str | None = None


class AttemptSeed(Strict):
    job_id: str
    created_at_millis: int
    spend: str


class Body(Strict):
    text: str
    pad_to: int | None = None


class StartStep(Strict):
    role: Role = "absent"
    store: bool = False
    user_id: str | None = None
    body: Body = Field(default_factory=Body)


class StopStep(Strict):
    role: Role = "absent"
    store: bool = False
    user_id: str | None = None
    job_id: str = ""


class Step(Strict):
    parse: Body | None = None
    start: StartStep | None = None
    stop: StopStep | None = None


class TargetView(Strict):
    target_type: TargetType
    target_id: str
    max_turns: int
    max_budget: str | None
    stopped_at_millis: int | None
    attempt_count: int | None
    spend: str | None
    target_alias: str | None
    key_name: str | None


class JobView(Strict):
    job_id: str
    router_name: str
    router_names: list[str]
    models: list[str]
    direction: Direction
    baseline_model: str | None
    judge_model: str
    shadow_percentage: str
    created_at_millis: int
    ends_at_millis: int
    stopped_by: str | None
    status: StatusText
    targets: list[TargetView]


class Succeeded(Strict):
    outcome: Literal["succeeded"] = "succeeded"
    status: int
    job: JobView


class Failed(Strict):
    outcome: Literal["failed"] = "failed"
    status: int
    detail: str


class Accepted(Strict):
    outcome: Literal["accepted"] = "accepted"


class Invalid(Strict):
    outcome: Literal["invalid"] = "invalid"


class OverLimit(Strict):
    outcome: Literal["over_limit"] = "over_limit"
    bytes: int
    limit: int


StepOutcome = Succeeded | Failed | Accepted | Invalid | OverLimit


class Output(Strict):
    steps: list[StepOutcome]
    jobs: list[str]


class InList(Strict):
    ids: list[str] = Field(alias="in")


class TargetFilter(Strict):
    target_type: TargetType
    target_id: InList


class ClaimFilter(Strict):
    direction: Direction
    stopped_at: None = None
    targets: list[TargetFilter] = Field(alias="OR")


class GroupFilter(Strict):
    group_id: str


class TokenFilter(Strict):
    token: InList


class TeamIdFilter(Strict):
    team_id: InList


class UserIdFilter(Strict):
    user_id: InList


class FunnelSeed(Strict):
    job_id: str


_CLAIM: Final = TypeAdapter(ClaimFilter)
_GROUP: Final = TypeAdapter(GroupFilter)
_TOKEN: Final = TypeAdapter(TokenFilter)
_TEAM: Final = TypeAdapter(TeamIdFilter)
_USER: Final = TypeAdapter(UserIdFilter)
_FUNNEL: Final = TypeAdapter(list[FunnelSeed])


class JobRow(Strict):
    id: str
    group_id: str
    target_type: TargetType
    target_id: str
    router_name: str
    router_names: list[str] = Field(default_factory=list)
    models: list[str] = Field(default_factory=list)
    direction: Direction
    baseline_model: str | None = None
    judge_model: str
    shadow_percentage: float
    max_turns: int
    max_budget: float | None = None
    created_at: datetime
    ends_at: datetime
    stopped_at: datetime | None = None
    stopped_by: str | None = None
    created_by: str | None = None


class Attempt(Strict):
    job_id: str
    created_at: datetime
    spend: float


class KeyRow(Strict):
    token: str
    key_alias: str | None = None
    key_name: str | None = None
    team_id: str | None = None


class TeamRow(Strict):
    team_id: str
    team_alias: str | None = None


class UserRow(Strict):
    user_id: str
    user_email: str | None = None


class KeyTable:
    def __init__(self, rows: Sequence[KeyRow]) -> None:
        self.rows: Sequence[KeyRow] = rows

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[KeyRow]:
        wanted = _TOKEN.validate_python(where).token.ids
        return [row for row in self.rows if row.token in wanted]


class TeamTable:
    def __init__(self, rows: Sequence[TeamRow]) -> None:
        self.rows: Sequence[TeamRow] = rows

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[TeamRow]:
        wanted = _TEAM.validate_python(where).team_id.ids
        return [row for row in self.rows if row.team_id in wanted]


class UserTable:
    def __init__(self, rows: Sequence[UserRow]) -> None:
        self.rows: Sequence[UserRow] = rows

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[UserRow]:
        wanted = _USER.validate_python(where).user_id.ids
        return [row for row in self.rows if row.user_id in wanted]


class JobTable:
    def __init__(self, store: "FakeStore") -> None:
        self.store: FakeStore = store

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[JobRow]:
        return self.store.matching(where)

    async def create_many(self, *, data: Sequence[Mapping[str, object]]) -> None:
        self.store.create(data)


class FunnelTable:
    def __init__(self, store: "FakeStore") -> None:
        self.store: FakeStore = store

    async def create_many(
        self, *, data: Sequence[Mapping[str, object]], skip_duplicates: bool = False
    ) -> None:
        _ = skip_duplicates
        seeded = _FUNNEL.validate_python(data)
        if self.store.seeds_fail:
            raise SeedError(f"the funnel seed for {len(seeded)} legs failed")


class SeedError(Exception):
    pass


class FakeStore:
    def __init__(self, rows: list[JobRow], attempts: list[Attempt], now: datetime) -> None:
        self.rows: list[JobRow] = rows
        self.attempts: list[Attempt] = attempts
        self.now: datetime = now
        self.seeds_fail: bool = False
        self.create_conflict: bool = False

    @classmethod
    def build(cls, seeds: Sequence[JobSeed], attempts: Sequence[AttemptSeed], now: datetime) -> "FakeStore":
        rows = [
            JobRow(
                id=seed.id,
                group_id=seed.group_id,
                target_type=seed.target_type,
                target_id=seed.target_id,
                router_name=seed.router_name,
                router_names=list(seed.router_names),
                models=list(seed.models),
                direction=seed.direction,
                baseline_model=seed.baseline_model,
                judge_model=seed.judge_model,
                shadow_percentage=float(seed.shadow_percentage),
                max_turns=seed.max_turns,
                max_budget=None if seed.max_budget is None else float(seed.max_budget),
                created_at=_instant(seed.created_at_millis),
                ends_at=_instant(seed.ends_at_millis),
                stopped_at=None if seed.stopped_at_millis is None else _instant(seed.stopped_at_millis),
                stopped_by=seed.stopped_by,
            )
            for seed in seeds
        ]
        seeded = [
            Attempt(
                job_id=seed.job_id,
                created_at=_instant(seed.created_at_millis),
                spend=float(seed.spend),
            )
            for seed in attempts
        ]
        return cls(rows, seeded, now)

    def matching(self, where: Mapping[str, object]) -> Sequence[JobRow]:
        try:
            group = _GROUP.validate_python(where).group_id
        except ValidationError:
            claim = _CLAIM.validate_python(where)
            return [
                row
                for row in self.rows
                if row.direction == claim.direction and row.stopped_at is None and _claimed(row, claim.targets)
            ]
        return [row for row in self.rows if row.group_id == group]

    def create(self, data: Sequence[Mapping[str, object]]) -> None:
        if self.create_conflict:
            raise UniqueViolationError({"code": "P2002"})
        self.rows.extend(JobRow.model_validate(item) for item in data)

    def seed_funnel(self, job_ids: Sequence[str]) -> None:
        _ = job_ids
        if self.seeds_fail:
            raise SeedError("the funnel table is unavailable")

    def attempt_count(self, row: JobRow) -> int:
        return sum(1 for attempt in self.attempts if attempt.job_id == row.id and _within(attempt, row))

    def attempt_spend(self, row: JobRow) -> float:
        return sum(attempt.spend for attempt in self.attempts if attempt.job_id == row.id and _within(attempt, row))

    def budget_open(self, row: JobRow) -> bool:
        return row.max_budget is None or self.attempt_spend(row) < row.max_budget

    def sweep(self, ids: Sequence[str], target_type: TargetType) -> None:
        for index, row in enumerate(self.rows):
            if row.target_type != target_type or row.target_id not in ids or row.stopped_at is not None:
                continue
            finished = row.ends_at <= self.now or self.attempt_count(row) >= row.max_turns
            if finished or not self.budget_open(row):
                self.rows[index] = row.model_copy(update={"stopped_at": self.now})

    def stop(self, job_id: str, operator: str, stamp_iso: str) -> int:
        stamp = datetime.fromisoformat(stamp_iso).replace(tzinfo=timezone.utc)
        group = [index for index, row in enumerate(self.rows) if row.group_id == job_id]
        if not group:
            return 0
        window_open = self.rows[group[0]].ends_at > self.now
        still_sampling = any(
            self.rows[index].stopped_at is None
            and self.attempt_count(self.rows[index]) < self.rows[index].max_turns
            and self.budget_open(self.rows[index])
            for index in group
        )
        if not window_open or not still_sampling:
            return 0
        for index in group:
            row = self.rows[index]
            if row.stopped_by is None and row.ends_at > self.now:
                stopped_at = stamp if row.stopped_at is None else row.stopped_at
                self.rows[index] = row.model_copy(update={"stopped_by": operator, "stopped_at": stopped_at})
        return 1

    def attempt_rows(self, ids: Sequence[str]) -> Sequence[Mapping[str, object]]:
        rows: list[Mapping[str, object]] = []
        for job_id in dict.fromkeys(ids):
            legs = [row for row in self.rows if row.id == job_id]
            if not legs:
                continue
            live = [attempt for attempt in self.attempts if attempt.job_id == job_id and _within(attempt, legs[0])]
            if not live:
                continue
            rows.append({"job_id": job_id, "attempt_count": len(live), "spend": sum(a.spend for a in live)})
        return rows

    def states(self) -> list[str]:
        return sorted(leg_state(row) for row in self.rows)


def _claimed(row: JobRow, targets: Sequence[TargetFilter]) -> bool:
    return any(row.target_type == target.target_type and row.target_id in target.target_id.ids for target in targets)


def _within(attempt: Attempt, row: JobRow) -> bool:
    return row.stopped_at is None or attempt.created_at <= row.stopped_at


def leg_state(row: JobRow) -> str:
    stopped = "" if row.stopped_at is None else str(_millis(row.stopped_at))
    stopped_by = row.stopped_by if row.stopped_by is not None else ""
    return "|".join(
        (
            row.id,
            row.group_id,
            row.target_type,
            row.target_id,
            ",".join(row.router_names),
            ",".join(row.models),
            row.direction,
            row.judge_model,
            str(row.max_turns),
            f"{row.shadow_percentage:.2f}",
            stopped,
            stopped_by,
        )
    )


class FakeDb:
    litellm_verificationtoken: KeyTable
    litellm_teamtable: TeamTable
    litellm_usertable: UserTable
    litellm_shadowevaljob: JobTable
    litellm_shadowevalfunnel: FunnelTable

    def __init__(self, store: FakeStore, rows: Rows) -> None:
        self.store: FakeStore = store
        self.litellm_verificationtoken = KeyTable(
            [
                KeyRow(token=key.token, key_alias=key.key_alias, key_name=key.key_name, team_id=key.team_id)
                for key in rows.keys
            ]
        )
        self.litellm_teamtable = TeamTable(
            [TeamRow(team_id=team.team_id, team_alias=team.team_alias) for team in rows.teams]
        )
        self.litellm_usertable = UserTable(
            [UserRow(user_id=user.user_id, user_email=user.user_email) for user in rows.users]
        )
        self.litellm_shadowevaljob = JobTable(store)
        self.litellm_shadowevalfunnel = FunnelTable(store)

    async def query_raw(self, query: str, *args: object) -> Sequence[Mapping[str, object]]:
        _ = query
        ids = _IDS.validate_python(args[0])
        return self.store.attempt_rows(ids)

    async def execute_raw(self, query: str, *args: object) -> int:
        if "SET stopped_at" in query:
            ids = _IDS.validate_python(args[0])
            target_type = _TARGET.validate_python(args[1])
            self.store.sweep(ids, target_type)
            return 0
        if "SET stopped_by" in query:
            job_id = _TEXT.validate_python(args[0])
            operator = _TEXT.validate_python(args[1])
            stamp = _TEXT.validate_python(args[2])
            return self.store.stop(job_id, operator, stamp)
        raise ProbeError("invalid_request", "the fake database cannot run this statement")


_IDS: Final = TypeAdapter(list[str])
_TARGET: Final = TypeAdapter(TargetType)
_TEXT: Final = TypeAdapter(str)


class FakePrisma:
    db: FakeDb

    def __init__(self, store: FakeStore, rows: Rows) -> None:
        self.db = FakeDb(store, rows)


class ProxyModule(ModuleType):
    llm_router: Router | None
    prisma_client: FakePrisma | None

    def __init__(self, name: str, router: Router | None, prisma: FakePrisma | None) -> None:
        super().__init__(name)
        self.llm_router = router
        self.prisma_client = prisma


def install_proxy(router: Router, prisma: FakePrisma | None) -> None:
    sys.modules[PROXY_MODULE] = ProxyModule(PROXY_MODULE, router, prisma)


def unconfigured() -> None:
    litellm.api_key = None
    litellm.anthropic_key = None
    _ = os.environ.pop("ANTHROPIC_API_KEY", None)
    _ = os.environ.pop("ANTHROPIC_AUTH_TOKEN", None)


def router_of(deployments: list[Deployment], aliases: dict[str, str | RouterModelGroupAliasItem]) -> Router:
    model_list: list[dict[str, JsonValue]] = [
        {"model_name": deployment.model_name, "litellm_params": deployment.litellm_params}
        for deployment in deployments
    ]
    return Router(model_list=model_list, model_group_alias=aliases)


def caller(role: Role, user_id: str | None) -> UserAPIKeyAuth:
    if role == "absent":
        return UserAPIKeyAuth(user_id=user_id)
    return UserAPIKeyAuth(user_role=LitellmUserRoles(role), user_id=user_id)


def body_bytes(body: Body) -> bytes:
    encoded = body.text.encode()
    total = len(encoded) if body.pad_to is None else body.pad_to
    limit = MAX_MESSAGE_BYTES + 1
    if total < len(encoded) or total > limit:
        raise ProbeError("invalid_request", f"a padded body must be between {len(encoded)} and {limit} bytes")
    return encoded + b" " * (total - len(encoded))


def parse(body: Body) -> Accepted | Invalid:
    try:
        _ = StartShadowEvalRequest.model_validate_json(body_bytes(body))
    except ValidationError:
        return Invalid()
    return Accepted()


def target_view(target: response_types.ShadowEvalJobTargetResponse) -> TargetView:
    return TargetView(
        target_type=target.target_type,
        target_id=target.target_id,
        max_turns=target.max_turns,
        max_budget=None if target.max_budget is None else f"{target.max_budget:.15f}",
        stopped_at_millis=None if target.stopped_at is None else _millis(target.stopped_at),
        attempt_count=target.attempt_count,
        spend=None if target.spend is None else f"{target.spend:.6f}",
        target_alias=target.target_alias,
        key_name=target.key_name,
    )


def job_view(response: response_types.ShadowEvalJobResponse) -> JobView:
    return JobView(
        job_id=response.job_id,
        router_name=response.router_name,
        router_names=list(response.router_names),
        models=list(response.models),
        direction=response.direction,
        baseline_model=response.baseline_model,
        judge_model=response.judge_model,
        shadow_percentage=f"{response.shadow_percentage:.2f}",
        created_at_millis=_millis(response.created_at),
        ends_at_millis=_millis(response.ends_at),
        stopped_by=response.stopped_by,
        status=response.status,
        targets=[target_view(target) for target in response.targets],
    )


async def run_start(store: FakeStore, rows: Rows, router: Router, step: StartStep) -> Succeeded | Failed:
    prisma = FakePrisma(store, rows) if step.store else None
    install_proxy(router, prisma)
    identity = caller(step.role, step.user_id)
    try:
        request = StartShadowEvalRequest.model_validate_json(body_bytes(step.body))
    except ValidationError:
        return Failed(status=BAD_REQUEST, detail="the body is not a start request")
    try:
        response = await start_shadow_eval(request, identity)
    except HTTPException as error:
        return Failed(status=error.status_code, detail=f"status {error.status_code}")
    return Succeeded(status=201, job=job_view(response))


async def run_stop(store: FakeStore, rows: Rows, router: Router, step: StopStep) -> Succeeded | Failed:
    prisma = FakePrisma(store, rows) if step.store else None
    install_proxy(router, prisma)
    identity = caller(step.role, step.user_id)
    try:
        response = await stop_shadow_eval_job(step.job_id, identity)
    except HTTPException as error:
        return Failed(status=error.status_code, detail=f"status {error.status_code}")
    return Succeeded(status=200, job=job_view(response))


async def outcome(store: FakeStore, rows: Rows, router: Router, step: Step) -> StepOutcome:
    if step.parse is not None:
        return parse(step.parse)
    if step.start is not None:
        return await run_start(store, rows, router, step.start)
    if step.stop is not None:
        return await run_stop(store, rows, router, step.stop)
    raise ProbeError("invalid_request", "a step names no action")


class ShadowEvalLifecycle(Operation):
    router: list[Deployment] = Field(default_factory=list)
    model_group_alias: dict[str, str | RouterModelGroupAliasItem] = Field(default_factory=dict)
    rows: Rows = Field(default_factory=Rows)
    jobs: list[JobSeed] = Field(default_factory=list)
    attempts: list[AttemptSeed] = Field(default_factory=list)
    now_millis: int = 0
    minted: list[str] = Field(default_factory=list)
    seeds_fail: bool = False
    create_conflict: bool = False
    steps: list[Step] = Field(default_factory=list)

    @override
    def run(self) -> Strict:
        global _NOW
        _NOW = _instant(self.now_millis)
        _MINTED[:] = [UUID(text) for text in self.minted]
        endpoints.uuid4 = _mint
        endpoints.datetime = FixedInstant
        response_types.datetime = FixedInstant
        unconfigured()
        router = router_of(self.router, self.model_group_alias)
        store = FakeStore.build(self.jobs, self.attempts, _NOW)
        store.seeds_fail = self.seeds_fail
        store.create_conflict = self.create_conflict
        steps: list[StepOutcome] = []
        for step in self.steps:
            steps.append(asyncio.run(outcome(store, self.rows, router, step)))
        return Output(steps=steps, jobs=store.states())
