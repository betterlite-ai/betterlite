import asyncio
import os
from typing import Literal, NotRequired, TypedDict

from litellm.models.team import LiteLLM_TeamTable, Member
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints.model_management_endpoints import (
    ModelManagementAuthChecks,
)
from litellm.proxy.utils import PrismaClient
from litellm.types.llms.bedrock import AwsSessionTag
from litellm.types.router import (
    Deployment,
    GenericLiteLLMParams,
    LiteLLM_Params,
    ModelInfo,
    updateDeployment,
)
from pydantic import BaseModel
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


class CallerInput(TypedDict):
    user_id: str | None
    user_role: UserRole | None
    team_id: str | None


class MemberInput(TypedDict):
    user_id: str | None
    role: Literal["admin", "user"]


class TeamInput(TypedDict):
    team_id: str
    organization_id: str | None
    members_with_roles: list[MemberInput]


# an absent key stays out of model_fields_set, which is how a patch leaves the field alone
class ParamsInput(TypedDict):
    litellm_credential_name: NotRequired[str | None]
    aws_session_tags: NotRequired[list[AwsSessionTag] | None]


class Step(TypedDict):
    function: Literal[
        "model_call",
        "create_write",
        "update_write",
        "team_action",
        "team_call",
        "attach_credential",
        "session_tags",
    ]
    caller: CallerInput
    team_id: str | None
    teams: list[TeamInput]
    allow_missing_team: bool
    patch: ParamsInput | None
    stored: ParamsInput | None
    null_detaches: bool
    salt_key: str


class Input(TypedDict):
    steps: list[Step]


class ErrorOut(TypedDict):
    status: int | None
    detail: Literal["message", "error"] | None
    error_type: str | None
    param: str | None
    code: str | None


class StepResult(TypedDict):
    value: bool | None
    error: ErrorOut | None
    team_reads: list[str]


# the HTTPException and ProxyException fields read by attribute, since mypy sees Any in both
class RaisedDetail(BaseModel):
    status_code: int
    detail: str | dict[str, str]


class RaisedDenial(BaseModel):
    type: str
    param: str | None
    code: str


# the LiteLLM_TeamTable rows the case holds, recording each team_id the check reads
class Table:
    def __init__(self, teams: list[LiteLLM_TeamTable]) -> None:
        self.teams = teams
        self.reads: list[str] = []

    async def find_unique(self, *, where: dict[str, str]) -> LiteLLM_TeamTable | None:
        team_id = where["team_id"]
        self.reads.append(team_id)
        found = [team for team in self.teams if team.team_id == team_id]
        return found[0] if found else None


class Db(PrismaWrapper):
    def __init__(self, table: Table) -> None:
        self.litellm_teamtable = table


class Client(PrismaClient):
    def __init__(self, db: Db) -> None:
        self.db = db


def run(input_value: Input) -> list[StepResult]:
    results: list[StepResult] = []
    for step in input_value["steps"]:
        os.environ["LITELLM_SALT_KEY"] = step["salt_key"]
        results.append(asyncio.run(run_step(step)))
    return results


async def run_step(step: Step) -> StepResult:
    table = Table([build_team(team) for team in step["teams"]])
    caller = build_caller(step["caller"])
    (outcome,) = await asyncio.gather(answer(step, caller, table), return_exceptions=True)
    if outcome is None:
        return {"value": True, "error": None, "team_reads": table.reads}
    error: ErrorOut
    match outcome:
        case HTTPException():
            raised = RaisedDetail.model_validate(outcome, from_attributes=True)
            shape: Literal["message", "error"]
            match raised.detail:
                case str():
                    shape = "message"
                case dict():
                    shape = "error"
            error = {
                "status": raised.status_code,
                "detail": shape,
                "error_type": None,
                "param": None,
                "code": None,
            }
        case BaseException():
            denial = RaisedDenial.model_validate(outcome, from_attributes=True)
            error = {
                "status": int(denial.code),
                "detail": None,
                "error_type": denial.type,
                "param": denial.param,
                "code": denial.code,
            }
    return {"value": None, "error": error, "team_reads": table.reads}


def build_caller(caller: CallerInput) -> UserAPIKeyAuth:
    role = caller["user_role"]
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=None if role is None else LitellmUserRoles(role),
        team_id=caller["team_id"],
    )


def build_team(team: TeamInput) -> LiteLLM_TeamTable:
    members = [
        Member(user_id=member["user_id"], user_email=None, role=member["role"])
        for member in team["members_with_roles"]
    ]
    return LiteLLM_TeamTable(
        team_id=team["team_id"],
        organization_id=team["organization_id"],
        members_with_roles=members,
    )


async def answer(step: Step, caller: UserAPIKeyAuth, table: Table) -> None:
    checks = ModelManagementAuthChecks
    client = Client(Db(table))
    team_id = step["team_id"]
    deployment = Deployment(
        model_name="model-write-fields",
        litellm_params=LiteLLM_Params(model="openai/gpt-4o-mini"),
        model_info=ModelInfo(team_id=team_id),
    )
    patch_input, stored_input = step["patch"], step["stored"]
    patch = None if patch_input is None else GenericLiteLLMParams(**patch_input)
    stored = None if stored_input is None else GenericLiteLLMParams(**stored_input)
    match step["function"]:
        case "model_call":
            await checks.can_user_make_model_call(
                model_params=deployment,
                user_api_key_dict=caller,
                prisma_client=client,
                premium_user=True,
                allow_missing_team=step["allow_missing_team"],
            )
        case "create_write" | "update_write":
            update = step["function"] == "update_write"
            await checks.can_user_make_model_call(
                model_params=deployment,
                user_api_key_dict=caller,
                prisma_client=client,
                premium_user=True,
                member_operation="update" if update else "create",
                incoming_model_params=updateDeployment() if update else None,
            )
        case "team_action":
            await checks.allow_team_model_action(
                model_params=updateDeployment(model_info=ModelInfo(team_id=team_id)),
                user_api_key_dict=caller,
                prisma_client=client,
                premium_user=True,
            )
        case "team_call":
            if team_id is None:
                raise ValueError("a team_call step names its team")
            known = [team for team in table.teams if team.team_id == team_id]
            checks.can_user_make_team_model_call(
                team_id=team_id,
                user_api_key_dict=caller,
                team_obj=known[0] if known else None,
                premium_user=True,
            )
        case "attach_credential":
            checks.can_user_attach_credential(
                litellm_params=patch,
                user_api_key_dict=caller,
                existing_litellm_params=stored,
                null_detaches=step["null_detaches"],
            )
        case "session_tags":
            checks.can_user_set_aws_session_tags(
                litellm_params=patch,
                user_api_key_dict=caller,
                existing_litellm_params=stored,
            )
