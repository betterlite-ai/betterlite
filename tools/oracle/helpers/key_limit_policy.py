import asyncio
from typing import Final, Literal, TypedDict

import litellm
from litellm.proxy._types import (
    GenerateKeyRequest,
    LiteLLM_OrganizationTable,
    LiteLLM_TeamTableCachedObj,
    UpdateKeyRequest,
    UserAPIKeyAuth,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints.key_management_endpoints import (
    _check_org_key_limits,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1897-L1937
    _check_project_key_limits,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1757-L1804
    _check_team_key_limits,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1716-L1749
    _enforce_upperbound_key_params,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1110-L1169
    _validate_caller_can_assign_key_org,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1865-L1894
)
from litellm.proxy.utils import PrismaClient
from litellm.types.proxy.management_endpoints.ui_sso import (
    LiteLLM_UpperboundKeyGenerateParams as UpperboundParams,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Row = dict[str, JsonValue]


class Request(TypedDict):
    kind: Literal["generate", "update"]
    body: Row


class UpperboundCall(TypedDict):
    check: Literal["upperbound"]
    request: Request
    params: Row | None
    fill_defaults: bool


class TeamCall(TypedDict):
    check: Literal["team"]
    request: Request
    team: Row
    keys: list[Row]


class ProjectCall(TypedDict):
    check: Literal["project"]
    request: Request
    project_id: str
    project: Row | None


class OrganizationCall(TypedDict):
    check: Literal["organization"]
    request: Request
    organization: Row
    keys: list[Row]


class AssignmentCall(TypedDict):
    check: Literal["assignment"]
    caller_user_id: str | None
    organization_id: str
    users: list[Row]


Call = UpperboundCall | TeamCall | ProjectCall | OrganizationCall | AssignmentCall


class KeyRow(BaseModel):
    token: str
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    metadata: dict[str, JsonValue] = Field(default_factory=dict)


class Membership(BaseModel):
    organization_id: str | None


class UserRow(BaseModel):
    organization_memberships: list[Membership]


class StoredRow(BaseModel):
    model_config = ConfigDict(extra="allow")


class Rejection(BaseModel):
    status_code: int
    detail: str | dict[str, str]


class Table:
    def __init__(self, rows: list[Row], model: type[BaseModel]) -> None:
        self.rows = rows
        self.model = model

    async def find_many(self, where: dict[str, str]) -> list[BaseModel]:
        field, value = next(iter(where.items()))
        return [self.model.model_validate(row) for row in self.rows if row.get(field) == value]

    async def find_unique(self, where: dict[str, str], **_: dict[str, bool]) -> BaseModel | None:
        found = await self.find_many(where)
        return found[0] if found else None


class FakeDb(PrismaWrapper):
    def __init__(self, tables: dict[str, Table]) -> None:
        self.tables = tables

    @override
    def __getattr__(self, name: str) -> Table:
        if name in self.tables:
            return self.tables[name]
        raise AttributeError(name)


class FakePrismaClient(PrismaClient):
    def __init__(self, tables: dict[str, Table]) -> None:
        self.db = FakeDb(tables)


def client(
    keys: list[Row] | None = None,
    project: Row | None = None,
    users: list[Row] | None = None,
) -> FakePrismaClient:
    tables = {
        "litellm_verificationtoken": Table([] if keys is None else keys, KeyRow),
        "litellm_projecttable": Table([] if project is None else [project], StoredRow),
        "litellm_usertable": Table([] if users is None else users, UserRow),
    }
    return FakePrismaClient(tables)


def data(request: Request) -> GenerateKeyRequest | UpdateKeyRequest:
    if request["kind"] == "generate":
        return GenerateKeyRequest.model_validate(request["body"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1289-L1332
    return UpdateKeyRequest.model_validate(request["body"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441-L1484


def values(request: GenerateKeyRequest | UpdateKeyRequest) -> dict[str, JsonValue]:
    return {
        "max_budget": request.max_budget,
        "max_parallel_requests": request.max_parallel_requests,
        "tpm_limit": request.tpm_limit,
        "rpm_limit": request.rpm_limit,
        "duration": request.duration,
        "budget_duration": request.budget_duration,
    }


def upperbound(call: UpperboundCall) -> JsonValue:
    request = data(call["request"])
    params = call["params"]
    litellm.upperbound_key_generate_params = (
        None if params is None else UpperboundParams.model_validate(params)
    )
    _enforce_upperbound_key_params(request, fill_defaults=call["fill_defaults"])
    return {"values": values(request)}


async def perform(call: Call) -> JsonValue:
    match call["check"]:
        case "upperbound":
            return upperbound(call)
        case "team":
            team = LiteLLM_TeamTableCachedObj.model_validate(call["team"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/team.py#L63-L83
            await _check_team_key_limits(team, data(call["request"]), client(keys=call["keys"]))
        case "project":
            await _check_project_key_limits(
                call["project_id"],
                data(call["request"]),
                client(project=call["project"]),
                UserApiKeyCache(),
            )
        case "organization":
            organization = LiteLLM_OrganizationTable.model_validate(call["organization"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/organization.py#L14-L29
            await _check_org_key_limits(
                organization, data(call["request"]), client(keys=call["keys"])
            )
        case "assignment":
            caller = UserAPIKeyAuth(user_id=call["caller_user_id"])
            await _validate_caller_can_assign_key_org(
                caller, call["organization_id"], client(users=call["users"])
            )
    return None


def run(input_value: Call) -> JsonValue:
    try:
        return asyncio.run(perform(input_value))
    except HTTPException as raised:
        rejection = TypeAdapter(Rejection).validate_python(raised, from_attributes=True)
        return {
            "status": rejection.status_code,
            "detail": JSON.validate_python(rejection.detail),
        }
