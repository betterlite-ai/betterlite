import asyncio
from datetime import datetime
from typing import Literal, TypedDict

import litellm
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints.auto_router_endpoints import _ATTEMPT_AGG_BY_LEG_SQL, _ATTEMPT_AGG_BY_MODEL_SQL, _ATTEMPT_AGG_BY_ROUTER_SQL, _ATTEMPT_AGG_BY_TIER_SQL, _ATTEMPT_COUNTS_SQL, _ATTEMPT_TOTALS_ROWS, _ATTEMPT_TOTALS_SQL, _FUNNEL_TOTALS_SQL, _LEG_ROWS, _group_response, _leg_attempt_counts, _query_raw, _shadow_eval_attempts, _shadow_eval_results, _with_target_labels  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/auto_router_endpoints.py#L1176-L1578
from litellm.proxy.utils import PrismaClient
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


class TokenWhere(BaseModel):
    token: InIds


class TeamWhere(BaseModel):
    team_id: InIds


class UserWhere(BaseModel):
    user_id: InIds


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
        ids = TokenWhere.model_validate(where).token.in_ids
        return [row for row in self.rows if row.token in ids]


class TeamTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = [TeamRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: JsonValue) -> list[TeamRow]:
        ids = TeamWhere.model_validate(where).team_id.in_ids
        return [row for row in self.rows if row.team_id in ids]


class UserTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = [UserRow.model_validate(row) for row in rows]

    async def find_many(self, *, where: JsonValue) -> list[UserRow]:
        ids = UserWhere.model_validate(where).user_id.in_ids
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
            return [row for row in self.call["counts"] if CountRow.model_validate(row).job_id in ids]
        if query == _ATTEMPT_TOTALS_SQL:
            return [self.call["totals"]]
        if query == _FUNNEL_TOTALS_SQL:
            return [self.call["funnel"]]
        if query == _ATTEMPT_AGG_BY_TIER_SQL:
            return self.call["by_tier"]
        if query == _ATTEMPT_AGG_BY_MODEL_SQL:
            return self.call["by_model"]
        if query == _ATTEMPT_AGG_BY_LEG_SQL:
            return self.call["by_leg"]
        if query == _ATTEMPT_AGG_BY_ROUTER_SQL:
            return self.call["by_router"]
        raise ValueError("the shadow eval detail read issued an unknown query")


class FakePrismaClient(PrismaClient):
    def __init__(self, call: Input) -> None:
        self.db = FakeDb(call)


async def answer(call: Input) -> dict[str, JsonValue]:
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
    response = labeled[0].model_copy(
        update={
            "judged_count": totals.judged_count,
            "error_count": totals.error_count,
            "judge_spend": round(totals.judge_spend, 6),
            "last_error": latest.error if latest is not None else None,
            "results": results,
            "targets": tuple(
                target.model_copy(update={"verdicts": verdicts.get((target.target_type, target.target_id))})
                for target in labeled[0].targets
            ),
        }
    )
    return {
        "response": DUMP.validate_json(response.model_dump_json(exclude={"router_name", "status"})),
    }


def run(input_value: Input) -> dict[str, JsonValue]:
    return asyncio.run(answer(input_value))
