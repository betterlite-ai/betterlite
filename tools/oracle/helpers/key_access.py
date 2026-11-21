import asyncio
import logging
from dataclasses import dataclass, field
from typing import Literal, TypedDict

import litellm
from fastapi import HTTPException
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy import proxy_server
from litellm.proxy._types import (
    GenerateKeyRequest,
    KeyManagementRoutes,
    LiteLLM_ObjectPermissionBase,
    RegenerateKeyRequest,
    UserAPIKeyAuth,
)
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints.key_management_endpoints import (
    common_key_access_checks,
    get_new_token,
    key_generation_check,
    raise_on_invalid_key_logging_config,
    validate_team_id_used_in_service_account_request,
)
from litellm.proxy.utils import PrismaClient
from litellm.types.utils import StandardKeyGenerationConfig
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

UserRole = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]
DetailShape = Literal["text", "error"]


class CallerInput(TypedDict):
    user_id: str | None
    user_role: UserRole | None
    team_id: str | None
    metadata: dict[str, str | None]


class MemberInput(TypedDict):
    user_id: str | None
    role: Literal["admin", "user"]


class TeamInput(TypedDict):
    team_id: str
    members_with_roles: list[MemberInput]
    team_member_permissions: list[str] | None


class RequestInput(TypedDict, total=False):
    user_id: str | None
    team_id: str | None
    key_alias: str | None
    duration: str | None
    models: list[str]
    access_group_ids: list[str] | None


class PermissionFields(TypedDict, total=False):
    mcp_servers: list[str] | None
    mcp_access_groups: list[str] | None
    mcp_tool_permissions: dict[str, list[str]] | None
    mcp_toolsets: list[str] | None
    blocked_tools: list[str] | None
    vector_stores: list[str] | None
    agents: list[str] | None
    agent_access_groups: list[str] | None
    models: list[str] | None
    search_tools: list[str] | None
    mcp_tool_search_enabled: bool | None
    skills: list[str] | None


class GeneralInput(TypedDict):
    disable_custom_api_keys: JsonValue


class LitellmInput(TypedDict):
    key_generation_settings: StandardKeyGenerationConfig | None


class ConfigInput(TypedDict):
    general_settings: GeneralInput
    litellm_settings: LitellmInput


class Step(TypedDict):
    function: Literal["custom_key", "generation", "common", "logging_metadata", "service_team", "object_permission"]
    config: ConfigInput
    caller: CallerInput
    team: TeamInput | None
    data: RequestInput
    route: str
    user_id: str | None
    custom_key: str | None
    metadata: dict[str, JsonValue] | None
    team_id: str | None
    database: bool
    teams: list[str]
    object_permission: PermissionFields | None


class Input(TypedDict):
    steps: list[Step]


class ErrorOut(TypedDict):
    status: int | None
    detail: DetailShape | None
    error_type: str | None
    param: str | None
    code: str | None


class StepResult(TypedDict):
    value: PermissionFields | None
    error: ErrorOut | None
    warnings: list[str]
    lookups: list[str]


class ErrorDetail(TypedDict):
    error: str


# the HTTPException fields the case compares, read by attribute
class RaisedHttp(BaseModel):
    status_code: int
    detail: str | ErrorDetail


# ProxyException's class type carries Any for mypy, so its fields are read the same way
class RaisedProxy(BaseModel):
    type: str
    param: str | None
    code: str


@dataclass
class Answer:
    permission: PermissionFields | None


@dataclass
class FoundTeam:
    team_id: str


# the LiteLLM_TeamTable the service-account check reads, recording every team id it is asked about
@dataclass
class CaseTeamTable:
    known: list[str]
    asked: list[str] = field(default_factory=list[str])

    async def find_unique(self, where: dict[str, str]) -> FoundTeam | None:
        self.asked.append(where["team_id"])
        found = where["team_id"] in self.known
        return FoundTeam(team_id=where["team_id"]) if found else None


class CaseDb(PrismaWrapper):
    def __init__(self, table: CaseTeamTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> CaseTeamTable:
        del name
        return self.table


class CasePrisma(PrismaClient):
    def __init__(self, table: CaseTeamTable) -> None:
        self.db = CaseDb(table)


class WarningRecords(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.lines: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append(record.getMessage())


def run(input_value: Input) -> list[StepResult]:
    results = [asyncio.run(run_step(step)) for step in input_value["steps"]]
    proxy_server.general_settings = {}
    litellm.key_generation_settings = None
    return results


async def run_step(step: Step) -> StepResult:
    general: dict[str, JsonValue] = {
        "disable_custom_api_keys": step["config"]["general_settings"]["disable_custom_api_keys"],
    }
    proxy_server.general_settings = general
    litellm.key_generation_settings = step["config"]["litellm_settings"]["key_generation_settings"]
    table = CaseTeamTable(known=step["teams"])
    records = WarningRecords()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.addHandler(records)
    try:
        (outcome,) = await asyncio.gather(call(step, table), return_exceptions=True)
    finally:
        logger.removeHandler(records)
    value: PermissionFields | None = None
    error: ErrorOut | None = None
    match outcome:
        case Answer():
            value = outcome.permission
        case BaseException():
            error = error_of(outcome)
    return {"value": value, "error": error, "warnings": records.lines, "lookups": table.asked}


async def call(step: Step, table: CaseTeamTable) -> Answer:
    caller = UserAPIKeyAuth.model_validate(step["caller"])
    data = GenerateKeyRequest.model_validate(step["data"])
    match step["function"]:
        case "custom_key":
            await get_new_token(RegenerateKeyRequest(new_key=step["custom_key"]))
        case "generation":
            team = None if step["team"] is None else LiteLLM_TeamTableCachedObj.model_validate(step["team"])
            route = KeyManagementRoutes(step["route"])
            key_generation_check(team_table=team, user_api_key_dict=caller, data=data, route=route)
        case "common":
            common_key_access_checks(caller, data, llm_router=None, premium_user=False, user_id=step["user_id"])
        case "logging_metadata":
            raise_on_invalid_key_logging_config(step["metadata"])
        case "service_team":
            client = CasePrisma(table) if step["database"] else None
            await validate_team_id_used_in_service_account_request(team_id=step["team_id"], prisma_client=client)
        case "object_permission":
            return Answer(permission=permission_dump(step["object_permission"]))
    return Answer(permission=None)


# _object_permission_to_dict has no public caller, so the case dumps the public model the same way it does
def permission_dump(permission: PermissionFields | None) -> PermissionFields | None:
    if permission is None:
        return None
    model = LiteLLM_ObjectPermissionBase.model_validate(permission)
    return TypeAdapter(PermissionFields).validate_json(model.model_dump_json(exclude_unset=True))


def error_of(outcome: BaseException) -> ErrorOut:
    error: ErrorOut = {"status": None, "detail": None, "error_type": None, "param": None, "code": None}
    match outcome:
        case HTTPException():
            raised = RaisedHttp.model_validate(outcome, from_attributes=True)
            error["status"] = raised.status_code
            error["detail"] = "error"
            match raised.detail:
                case str():
                    error["detail"] = "text"
        case ValueError():
            pass
        case BaseException():
            denial = RaisedProxy.model_validate(outcome, from_attributes=True)
            error["status"] = int(denial.code)
            error["error_type"] = denial.type
            error["param"] = denial.param
            error["code"] = denial.code
    return error
