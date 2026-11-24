import asyncio
from dataclasses import dataclass
from typing import Final, Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.models.team import LiteLLM_TeamTable
from litellm.models.user import LiteLLM_UserTable
from litellm.models.verification_token import LiteLLM_VerificationToken
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints.key_management_endpoints import (
    _apply_non_admin_alias_scope,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6687-L6718
    _fetch_user_team_objects,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6379-L6394
    _get_admin_team_ids_from_objects,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6397-L6402
    _get_member_team_ids_from_objects,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6425-L6437
    _get_team_ids_with_key_list_permission_from_objects,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6405-L6422
    validate_key_list_check,
)
from litellm.proxy.utils import PrismaClient
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from starlette.exceptions import HTTPException

UserRole = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]

CALLER: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)
USER: Final[TypeAdapter[LiteLLM_UserTable]] = TypeAdapter(LiteLLM_UserTable)
KEY: Final[TypeAdapter[LiteLLM_VerificationToken]] = TypeAdapter(LiteLLM_VerificationToken)
TEAM: Final[TypeAdapter[LiteLLM_TeamTable]] = TypeAdapter(LiteLLM_TeamTable)
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class CallerInput(TypedDict):
    user_id: str | None
    user_role: UserRole | None


class SelectorsInput(TypedDict):
    user_id: str | None
    team_id: str | None
    organization_id: str | None
    key_alias: str | None
    key_hash: str | None


class ValidateCall(TypedDict):
    function: Literal["validate"]
    caller: CallerInput
    selectors: SelectorsInput
    user_row: dict[str, JsonValue] | None
    key_row: dict[str, JsonValue] | None
    key_fails: bool


class TeamsCall(TypedDict):
    function: Literal["teams"]
    user: dict[str, JsonValue] | None
    teams: list[dict[str, JsonValue]] | None
    teams_fail: bool


class ProjectionsCall(TypedDict):
    function: Literal["projections"]
    caller: CallerInput
    teams: list[dict[str, JsonValue]]


class AliasScopeCall(TypedDict):
    function: Literal["alias_scope"]
    caller: CallerInput
    user_row: dict[str, JsonValue] | None
    query_params: list[JsonValue]
    where_parts: list[str]


Call: TypeAlias = ValidateCall | TeamsCall | ProjectionsCall | AliasScopeCall


class Input(TypedDict):
    calls: list[Call]


class ErrorOut(TypedDict):
    status: int | None
    error_type: str | None
    param: str | None
    code: str | None


# the exception fields the case compares, read by attribute because the class carries Any
class RaisedProxy(BaseModel):
    type: str
    param: str | None
    code: str


class TeamWhere(TypedDict):
    team_id: dict[str, list[str]]


@dataclass
class CaseUserTable:
    row: LiteLLM_UserTable | None
    effects: list[str]

    async def find_unique(
        self,
        *,
        where: dict[str, str],
        include: dict[str, bool] | None = None,
    ) -> LiteLLM_UserTable | None:
        self.effects.append(f"user {where['user_id']}")
        del include
        return self.row


@dataclass
class CaseKeyTable:
    row: LiteLLM_VerificationToken | None
    fails: bool
    effects: list[str]

    async def find_unique(self, *, where: dict[str, str]) -> LiteLLM_VerificationToken | None:
        self.effects.append("key")
        if self.fails:
            raise RuntimeError("the database is unavailable")
        if self.row is None or self.row.token != where["token"]:
            return None
        return self.row


@dataclass
class CaseTeamTable:
    rows: list[dict[str, JsonValue]] | None
    fails: bool
    effects: list[str]

    async def find_many(self, *, where: TeamWhere) -> list[LiteLLM_TeamTable] | None:
        self.effects.append(f"teams {','.join(where['team_id']['in'])}")
        if self.fails:
            raise RuntimeError("the database is unavailable")
        if self.rows is None:
            return None
        return [TEAM.validate_python(row) for row in self.rows]


class CaseDb(PrismaWrapper):
    def __init__(self, user: CaseUserTable, key: CaseKeyTable, team: CaseTeamTable) -> None:
        self.litellm_usertable = user
        self.litellm_verificationtoken = key
        self.litellm_teamtable = team


class CasePrisma(PrismaClient):
    def __init__(self, database: CaseDb) -> None:
        self.db = database


def _caller_record(caller: CallerInput) -> UserAPIKeyAuth:
    fields = {"user_id": caller["user_id"], "user_role": caller["user_role"]}
    return CALLER.validate_python(fields)


def _row_summary(row: LiteLLM_UserTable) -> JsonValue:
    memberships = row.organization_memberships
    organization_ids = (
        None if memberships is None else [member.organization_id for member in memberships]
    )
    summary = {"user_id": row.user_id, "teams": row.teams, "organization_ids": organization_ids}
    return _JSON.validate_python(summary)


async def _validate_answer(call: ValidateCall, effects: list[str]) -> JsonValue:
    caller = _caller_record(call["caller"])
    user_row = None if call["user_row"] is None else USER.validate_python(call["user_row"])
    key_row = None if call["key_row"] is None else KEY.validate_python(call["key_row"])
    client = CasePrisma(
        CaseDb(
            CaseUserTable(row=user_row, effects=effects),
            CaseKeyTable(row=key_row, fails=call["key_fails"], effects=effects),
            CaseTeamTable(rows=[], fails=False, effects=effects),
        )
    )
    selectors = call["selectors"]
    row = await validate_key_list_check(
        user_api_key_dict=caller,
        user_id=selectors["user_id"],
        team_id=selectors["team_id"],
        organization_id=selectors["organization_id"],
        key_alias=selectors["key_alias"],
        key_hash=selectors["key_hash"],
        prisma_client=client,
    )
    return None if row is None else _row_summary(row)


async def _teams_answer(call: TeamsCall, effects: list[str]) -> JsonValue:
    user = None if call["user"] is None else USER.validate_python(call["user"])
    client = CasePrisma(
        CaseDb(
            CaseUserTable(row=None, effects=effects),
            CaseKeyTable(row=None, fails=False, effects=effects),
            CaseTeamTable(rows=call["teams"], fails=call["teams_fail"], effects=effects),
        )
    )
    teams = await _fetch_user_team_objects(user, client)
    team_ids: list[str] = [team.team_id for team in teams]
    return _JSON.validate_python(team_ids)


def _projections_answer(call: ProjectionsCall) -> JsonValue:
    caller = _caller_record(call["caller"])
    teams = [TEAM.validate_python(row) for row in call["teams"]]
    admins = _get_admin_team_ids_from_objects(caller, teams)
    permitted = _get_team_ids_with_key_list_permission_from_objects(caller, teams)
    members = _get_member_team_ids_from_objects(caller, teams)
    projection = {"admins": admins, "list_permission": permitted, "members": members}
    return _JSON.validate_python(projection)


async def _alias_scope_answer(call: AliasScopeCall, effects: list[str]) -> JsonValue:
    params = list(call["query_params"])
    parts = list(call["where_parts"])
    user_row = None if call["user_row"] is None else USER.validate_python(call["user_row"])
    await _apply_non_admin_alias_scope(
        _caller_record(call["caller"]),
        CasePrisma(
            CaseDb(
                CaseUserTable(row=user_row, effects=effects),
                CaseKeyTable(row=None, fails=False, effects=effects),
                CaseTeamTable(rows=[], fails=False, effects=effects),
            )
        ),
        cast("list[object]", params),  # LiteLLM types query_params as list[object]: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L6687-L6692
        parts,
    )
    answer = {"params": params, "where_parts": parts}
    return _JSON.validate_python(answer)


def _error_of(
    outcome: HTTPException | ProxyException | RuntimeError | ValidationError,
) -> JsonValue:
    error: ErrorOut = {"status": None, "error_type": None, "param": None, "code": None}
    match outcome:
        case HTTPException():
            error["status"] = outcome.status_code
        case ProxyException():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
            denial = RaisedProxy.model_validate(outcome, from_attributes=True)
            error["status"] = int(denial.code)
            error["error_type"] = denial.type
            error["param"] = denial.param
            error["code"] = denial.code
        case RuntimeError() | ValidationError():
            pass
    return _JSON.validate_python(error)


def _envelope(value: JsonValue, error: JsonValue, effects: list[str]) -> JsonValue:
    warnings: list[str] = []
    envelope = {"value": value, "error": error, "warnings": warnings, "effects": effects}
    return _JSON.validate_python(envelope)


async def _call_outcome(call: Call) -> JsonValue:
    effects: list[str] = []
    value: JsonValue = None
    try:
        match call["function"]:
            case "validate":
                value = await _validate_answer(call, effects)
            case "teams":
                value = await _teams_answer(call, effects)
            case "projections":
                value = _projections_answer(call)
            case "alias_scope":
                value = await _alias_scope_answer(call, effects)
    except (HTTPException, ProxyException, RuntimeError, ValidationError) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return _envelope(None, _error_of(error), effects)
    return _envelope(value, None, effects)


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    outcomes: list[JsonValue] = [asyncio.run(_call_outcome(call)) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
