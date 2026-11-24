import asyncio
import sys
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime, tzinfo
from types import ModuleType
from typing import Annotated, ClassVar, Final, Literal
from urllib.parse import parse_qsl

from fastapi import HTTPException
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.management_endpoints.auto_router_endpoints import (
    get_shadow_eval_job,
    list_shadow_eval_jobs,
)
from litellm.types.management_endpoints import auto_router_endpoints as response_types
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
MAX_QUERY_BYTES: Final = 1 << 20
UNPROCESSABLE: Final = 422
DUMP: Final = TypeAdapter(dict[str, JsonValue])
Role = Literal["absent", "proxy_admin", "proxy_admin_viewer", "internal_user", "customer"]
TargetType = Literal["key", "team", "user"]

_NOW: list[datetime] = [datetime(2023, 11, 14, tzinfo=UTC)]


class FixedInstant(datetime):
    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> "FixedInstant":
        _ = tz
        return cls.fromtimestamp(_NOW[0].timestamp(), UTC)


def instant(millis: int) -> datetime:
    return datetime.fromtimestamp(millis / 1000, UTC)


class KeyRow(Strict):
    token: str = Field(alias="target_id")
    key_alias: str | None = Field(default=None, alias="target_alias")
    key_name: str | None = None


class TeamRow(Strict):
    team_id: str = Field(alias="target_id")
    team_alias: str | None = Field(default=None, alias="target_alias")


class UserRow(Strict):
    user_id: str = Field(alias="target_id")
    user_email: str | None = Field(default=None, alias="target_alias")


class Rows(Strict):
    keys: list[dict[str, JsonValue]] = Field(default_factory=list)
    teams: list[dict[str, JsonValue]] = Field(default_factory=list)
    users: list[dict[str, JsonValue]] = Field(default_factory=list)


class ListStep(Strict):
    role: Role = "absent"
    store: bool = False
    query: str = ""
    pad_to: int | None = None


class DetailStep(Strict):
    role: Role = "absent"
    store: bool = False
    job_id: str = ""


class Step(Strict):
    list_step: ListStep | None = Field(default=None, alias="list")
    detail: DetailStep | None = None


class Listed(Strict):
    outcome: Literal["listed"] = "listed"
    jobs: list[dict[str, JsonValue]]


class Detailed(Strict):
    outcome: Literal["detailed"] = "detailed"
    job: dict[str, JsonValue]


class Failed(Strict):
    outcome: Literal["failed"] = "failed"
    status: int


class OverLimit(Strict):
    outcome: Literal["over_limit"] = "over_limit"
    bytes: int
    limit: int


StepOutcome = Annotated[Listed | Detailed | Failed | OverLimit, Field(discriminator="outcome")]


class Output(Strict):
    steps: list[StepOutcome]


class InIds(Strict):
    ids: list[str] = Field(alias="in")


class GroupWhere(Strict):
    group_id: str


class TokenWhere(Strict):
    token: InIds


class TeamWhere(Strict):
    team_id: InIds


class UserWhere(Strict):
    user_id: InIds


class ErrorWhere(Strict):
    job_id: InIds
    outcome: Literal["error"]


class ErrorOrder(Strict):
    created_at: Literal["desc"]


class AttemptRow(Strict):
    error: str | None = None


_GROUP: Final = TypeAdapter(GroupWhere)
_TOKEN: Final = TypeAdapter(TokenWhere)
_TEAM: Final = TypeAdapter(TeamWhere)
_USER: Final = TypeAdapter(UserWhere)
_ERROR_WHERE: Final = TypeAdapter(ErrorWhere)
_ERROR_ORDER: Final = TypeAdapter(ErrorOrder)
_IDS: Final = TypeAdapter(list[str])
_INT: Final = TypeAdapter(int)
_TEXT: Final = TypeAdapter(str)
_TARGET_TYPE: Final = TypeAdapter(TargetType)


class KeyTable:
    def __init__(self, rows: Sequence[dict[str, JsonValue]]) -> None:
        self.rows: list[KeyRow] = [KeyRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[KeyRow]:
        wanted = _TOKEN.validate_python(where).token.ids
        return [row for row in self.rows if row.token in wanted]


class TeamTable:
    def __init__(self, rows: Sequence[dict[str, JsonValue]]) -> None:
        self.rows: list[TeamRow] = [TeamRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[TeamRow]:
        wanted = _TEAM.validate_python(where).team_id.ids
        return [row for row in self.rows if row.team_id in wanted]


class UserTable:
    def __init__(self, rows: Sequence[dict[str, JsonValue]]) -> None:
        self.rows: list[UserRow] = [UserRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[UserRow]:
        wanted = _USER.validate_python(where).user_id.ids
        return [row for row in self.rows if row.user_id in wanted]


class JobTable:
    def __init__(self, rows: Sequence[dict[str, JsonValue]]) -> None:
        self.rows: Sequence[dict[str, JsonValue]] = rows

    async def find_many(self, *, where: Mapping[str, object]) -> Sequence[dict[str, JsonValue]]:
        group = _GROUP.validate_python(where).group_id
        return [row for row in self.rows if row["group_id"] == group]


class AttemptTable:
    def __init__(self, rows: Sequence[dict[str, JsonValue]]) -> None:
        self.rows: Sequence[dict[str, JsonValue]] = rows

    async def find_first(
        self, *, where: Mapping[str, object], order: Mapping[str, str]
    ) -> AttemptRow | None:
        wanted = _ERROR_WHERE.validate_python(where).job_id.ids
        _ = _ERROR_ORDER.validate_python(order)
        newest: dict[str, JsonValue] | None = None
        for row in self.rows:
            if row["job_id"] not in wanted:
                continue
            if newest is None or _TEXT.validate_python(row["created_at"]) > _TEXT.validate_python(
                newest["created_at"]
            ):
                newest = row
        if newest is None:
            return None
        return AttemptRow.model_validate({"error": newest.get("error")})


class FakeStore:
    def __init__(self, payload: "ShadowEvalResults") -> None:
        self.jobs: list[dict[str, JsonValue]] = list(payload.jobs)
        self.counts: list[dict[str, JsonValue]] = list(payload.counts)
        self.totals: dict[str, JsonValue] | None = payload.totals
        self.errors: list[dict[str, JsonValue]] = list(payload.errors)
        self.by_tier: list[dict[str, JsonValue]] = list(payload.by_tier)
        self.by_model: list[dict[str, JsonValue]] = list(payload.by_model)
        self.by_leg: list[dict[str, JsonValue]] = list(payload.by_leg)
        self.by_router: list[dict[str, JsonValue]] = list(payload.by_router)
        self.funnel: dict[str, JsonValue] | None = payload.funnel
        self.rows: Rows = payload.rows

    def list_rows(self, args: Sequence[object], *, filtered: bool) -> Sequence[Mapping[str, object]]:
        limit = _INT.validate_python(args[0])
        target: tuple[TargetType, str] | None = None
        if filtered:
            target = (_TARGET_TYPE.validate_python(args[1]), _TEXT.validate_python(args[2]))
        groups: dict[str, list[dict[str, JsonValue]]] = {}
        for row in self.jobs:
            if target is None or (row["target_type"] == target[0] and row["target_id"] == target[1]):
                groups.setdefault(_TEXT.validate_python(row["group_id"]), []).append(row)
        ordered = sorted(groups)
        ordered.sort(
            key=lambda group: max(
                _TEXT.validate_python(row["created_at"]) for row in groups[group]
            ),
            reverse=True,
        )
        selected = set(ordered[:limit])
        return [row for row in self.jobs if _TEXT.validate_python(row["group_id"]) in selected]

    def static_rows(
        self, query: str, args: Sequence[object]
    ) -> Sequence[Mapping[str, object]] | None:
        if "AS attempt_count" in query:
            return self.count_rows(args)
        if "judged_count" in query:
            return [] if self.totals is None else [self.totals]
        if "legs_with_rows" in query:
            return [] if self.funnel is None else [self.funnel]
        groups = (
            ("COALESCE(tier", self.by_tier),
            ("COALESCE(real_model", self.by_model),
            ("job_id AS grp", self.by_leg),
            ("COALESCE(a.router_name", self.by_router),
        )
        for marker, rows in groups:
            if marker in query:
                return rows
        return None

    def count_rows(self, args: Sequence[object]) -> Sequence[Mapping[str, object]]:
        wanted = _IDS.validate_python(args[0])
        return [row for row in self.counts if _TEXT.validate_python(row["job_id"]) in wanted]


class FakeDb:
    litellm_verificationtoken: KeyTable
    litellm_teamtable: TeamTable
    litellm_usertable: UserTable
    litellm_shadowevaljob: JobTable
    litellm_shadowevalattempt: AttemptTable

    def __init__(self, store: FakeStore) -> None:
        self.store: FakeStore = store
        self.litellm_verificationtoken = KeyTable(store.rows.keys)
        self.litellm_teamtable = TeamTable(store.rows.teams)
        self.litellm_usertable = UserTable(store.rows.users)
        self.litellm_shadowevaljob = JobTable(store.jobs)
        self.litellm_shadowevalattempt = AttemptTable(store.errors)

    async def query_raw(self, query: str, *args: object) -> Sequence[Mapping[str, object]]:
        if "ORDER BY MAX(created_at) DESC LIMIT $1::int" in query:
            return self.store.list_rows(args, filtered="target_type = $2" in query)
        rows = self.store.static_rows(query, args)
        if rows is None:
            raise ProbeError("invalid_request", "the fake database cannot run this statement")
        return rows


class FakePrisma:
    db: FakeDb

    def __init__(self, store: FakeStore) -> None:
        self.db = FakeDb(store)


class ProxyModule(ModuleType):
    prisma_client: FakePrisma | None

    def __init__(self, name: str, prisma: FakePrisma | None) -> None:
        super().__init__(name)
        self.prisma_client = prisma


def install_proxy(prisma: FakePrisma | None) -> None:
    sys.modules[PROXY_MODULE] = ProxyModule(PROXY_MODULE, prisma)


def caller(role: Role) -> UserAPIKeyAuth:
    if role == "absent":
        return UserAPIKeyAuth(user_id=None)
    return UserAPIKeyAuth(user_role=LitellmUserRoles(role))


class ListQuery(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    target_type: TargetType | None = None
    target_id: str | None = None
    limit: int = Field(default=50, ge=1, le=200)


def padded_query(step: ListStep) -> str:
    encoded = step.query.encode()
    total = len(encoded) if step.pad_to is None else step.pad_to
    limit = MAX_QUERY_BYTES + 1
    if total < len(encoded) or total > limit:
        raise ProbeError(
            "invalid_request",
            f"a padded query must be between {len(encoded)} and {limit} bytes",
        )
    return step.query + "x" * (total - len(encoded))


def parse_query(step: ListStep) -> ListQuery | None:
    values = dict(parse_qsl(padded_query(step), keep_blank_values=True))
    try:
        return ListQuery.model_validate(values)
    except ValidationError:
        return None


async def run_list(store: FakeStore, step: ListStep) -> StepOutcome:
    install_proxy(FakePrisma(store) if step.store else None)
    query = parse_query(step)
    if query is None:
        return Failed(status=UNPROCESSABLE)
    try:
        jobs = await list_shadow_eval_jobs(
            caller(step.role),
            target_type=query.target_type,
            target_id=query.target_id,
            limit=query.limit,
        )
    except HTTPException as error:
        return Failed(status=error.status_code)
    return Listed(jobs=[DUMP.validate_json(job.model_dump_json()) for job in jobs])


async def run_detail(store: FakeStore, step: DetailStep) -> StepOutcome:
    install_proxy(FakePrisma(store) if step.store else None)
    try:
        job = await get_shadow_eval_job(step.job_id, caller(step.role))
    except HTTPException as error:
        return Failed(status=error.status_code)
    return Detailed(job=DUMP.validate_json(job.model_dump_json()))


async def outcome(store: FakeStore, step: Step) -> StepOutcome:
    if step.list_step is not None:
        return await run_list(store, step.list_step)
    if step.detail is not None:
        return await run_detail(store, step.detail)
    raise ProbeError("invalid_request", "a step names no action")


async def scenario(store: FakeStore, steps: Sequence[Step]) -> Output:
    return Output(steps=[await outcome(store, step) for step in steps])


class ShadowEvalResults(Operation):
    now_millis: int = 0
    jobs: list[dict[str, JsonValue]] = Field(default_factory=list)
    counts: list[dict[str, JsonValue]] = Field(default_factory=list)
    totals: dict[str, JsonValue] | None = None
    errors: list[dict[str, JsonValue]] = Field(default_factory=list)
    by_tier: list[dict[str, JsonValue]] = Field(default_factory=list)
    by_model: list[dict[str, JsonValue]] = Field(default_factory=list)
    by_leg: list[dict[str, JsonValue]] = Field(default_factory=list)
    by_router: list[dict[str, JsonValue]] = Field(default_factory=list)
    funnel: dict[str, JsonValue] | None = None
    rows: Rows = Field(default_factory=Rows)
    steps: list[Step] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        _NOW[0] = instant(self.now_millis)
        response_types.datetime = FixedInstant
        store = FakeStore(self)
        return asyncio.run(scenario(store, self.steps))
