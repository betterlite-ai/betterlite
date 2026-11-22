import asyncio
from datetime import datetime
from typing import Literal, TypedDict

import litellm
from litellm.proxy.db.prisma_client import PrismaWrapper

from litellm.proxy.management_endpoints.auto_router_endpoints import _ATTEMPT_AGG_BY_LEG_SQL, _ATTEMPT_AGG_BY_MODEL_SQL, _ATTEMPT_AGG_BY_ROUTER_SQL, _ATTEMPT_AGG_BY_TIER_SQL, _ATTEMPT_COUNTS_SQL, _ATTEMPT_TOTALS_ROWS, _ATTEMPT_TOTALS_SQL, _FUNNEL_TOTALS_SQL, _LEG_ROWS, _group_response, _leg_attempt_counts, _query_raw, _shadow_eval_attempts, _shadow_eval_results, _with_target_labels  # noqa: E501  # isort: skip  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/auto_router_endpoints.py#L1176-L1578
from litellm.proxy.utils import PrismaClient
from litellm.types.management_endpoints.auto_router_endpoints import ShadowEvalJobResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

DUMP: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    group_id: str
    legs: list[dict[str, JsonValue]]
    counts: list[dict[str, JsonValue]]
    totals: dict[str, JsonValue]
    errors: list[dict[str, JsonValue]]
    by_tier: list[dict[str, JsonValue]]
    by_model: list[dict[str, JsonValue]]
    by_leg: list[dict[str, JsonValue]]
    by_router: list[dict[str, JsonValue]]
    funnel: dict[str, JsonValue]
    keys: list[dict[str, JsonValue]]
    teams: list[dict[str, JsonValue]]
    users: list[dict[str, JsonValue]]


class InIds(BaseModel):
    in_ids: list[str] = Field(alias="in")


class ErrorWhere(BaseModel):
    job_id: InIds
    outcome: Literal["error"]


class ErrorOrder(BaseModel):
    created_at: Literal["desc"]


class KeyRow(BaseModel):
    token: str
    key_alias: str | None
    key_name: str | None


class TeamRow(BaseModel):
    team_id: str
    team_alias: str | None


class UserRow(BaseModel):
    user_id: str
    user_email: str | None


class CountRow(BaseModel):
    job_id: str
    attempt_count: int
    spend: float


class ErrorRow(BaseModel):
    job_id: str
    error: str | None
    created_at: datetime


class KeyTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = [KeyRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: JsonValue) -> list[KeyRow]:
        ids = TypeAdapter(dict[str, InIds]).validate_python(where)["token"].in_ids
        return [row for row in self.rows if row.token in ids]


class TeamTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = [TeamRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: JsonValue) -> list[TeamRow]:
        ids = TypeAdapter(dict[str, InIds]).validate_python(where)["team_id"].in_ids
        return [row for row in self.rows if row.team_id in ids]


class UserTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = [UserRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: JsonValue) -> list[UserRow]:
        ids = TypeAdapter(dict[str, InIds]).validate_python(where)["user_id"].in_ids
        return [row for row in self.rows if row.user_id in ids]


def newest(rows: list[ErrorRow]) -> ErrorRow | None:
    newest_row: ErrorRow | None = None
    for row in rows:
        if newest_row is None or row.created_at > newest_row.created_at:
            newest_row = row
    return newest_row


class AttemptTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = [ErrorRow.model_validate(row) for row in rows]

    async def find_first(self, *, where: JsonValue, order: JsonValue) -> ErrorRow | None:
        ids = ErrorWhere.model_validate(where).job_id.in_ids
        _ = ErrorOrder.model_validate(order)
        return newest([row for row in self.rows if row.job_id in ids])


class FakeDb(PrismaWrapper):
    def __init__(self, call: Input) -> None:
        self.call = call
        self.litellm_verificationtoken = KeyTable(call["keys"])
        self.litellm_teamtable = TeamTable(call["teams"])
        self.litellm_usertable = UserTable(call["users"])
        self.litellm_shadowevalattempt = AttemptTable(call["errors"])

    async def query_raw(self, query: str, *args: JsonValue) -> list[dict[str, JsonValue]]:
        ids = TypeAdapter(list[str]).validate_python(args[0])
        if query == _ATTEMPT_COUNTS_SQL:
            counts = self.call["counts"]
            return [row for row in counts if CountRow.model_validate(row).job_id in ids]
        rows = {
            _ATTEMPT_TOTALS_SQL: [self.call["totals"]],
            _FUNNEL_TOTALS_SQL: [self.call["funnel"]],
            _ATTEMPT_AGG_BY_TIER_SQL: self.call["by_tier"],
            _ATTEMPT_AGG_BY_MODEL_SQL: self.call["by_model"],
            _ATTEMPT_AGG_BY_LEG_SQL: self.call["by_leg"],
            _ATTEMPT_AGG_BY_ROUTER_SQL: self.call["by_router"],
        }
        if query not in rows:
            raise ValueError("the shadow eval detail read issued an unknown query")
        return rows[query]


class FakePrismaClient(PrismaClient):
    def __init__(self, call: Input) -> None:
        self.db = FakeDb(call)


async def answer(call: Input) -> ShadowEvalJobResponse:
    legs = _LEG_ROWS.validate_python(call["legs"])
    client = FakePrismaClient(call)
    leg_ids = [leg.id for leg in legs]
    counts = await _leg_attempt_counts(client, legs)
    totals = _ATTEMPT_TOTALS_ROWS.validate_python(
        await _query_raw(client, _ATTEMPT_TOTALS_SQL, leg_ids)
    )[0]
    latest = await _shadow_eval_attempts(client).find_first(
        where={"job_id": {"in": leg_ids}, "outcome": "error"},
        order={"created_at": "desc"},
    )
    labeled = await _with_target_labels(client, (_group_response(call["group_id"], legs, counts),))
    results, verdicts = await _shadow_eval_results(client, legs)
    response = labeled[0].model_copy()
    response.judged_count = totals.judged_count
    response.error_count = totals.error_count
    response.judge_spend = round(totals.judge_spend, 6)
    response.last_error = latest.error if latest is not None else None
    response.results = results
    for target in response.targets:
        target.verdicts = verdicts.get((target.target_type, target.target_id))
    return response


def run(input_value: Input) -> dict[str, JsonValue]:
    response = asyncio.run(answer(input_value))
    dumped = DUMP.validate_json(response.model_dump_json(exclude={"router_name", "status"}))
    return {"response": dumped}
