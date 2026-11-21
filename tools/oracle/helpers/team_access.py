import asyncio
from dataclasses import dataclass, field
from typing import Literal, TypedDict

from fastapi import HTTPException
from litellm.models.team import LiteLLM_TeamTableCachedObj, Member
from litellm.proxy._types import LitellmUserRoles, ProxyException, UserAPIKeyAuth
from litellm.proxy.management.teams.access import (
    TeamAccess,
    TeamRole,
    is_team_admin,
    team_access_denied,
)
from litellm.proxy.management_helpers.team_member_permission_checks import (
    TeamMemberPermissionChecks,
)

UserRole = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]
Answer = bool | str | list[str] | None


class CallerMetadata(TypedDict):
    service_account_id: str | None


class CallerInput(TypedDict):
    user_id: str | None
    user_role: UserRole | None
    team_id: str | None
    metadata: CallerMetadata


class MemberInput(TypedDict):
    user_id: str | None
    user_email: str | None
    role: Literal["admin", "user"]


class TeamInput(TypedDict):
    team_id: str
    organization_id: str | None
    members_with_roles: list[MemberInput]
    team_member_permissions: list[str] | None


class Step(TypedDict):
    function: Literal[
        "allows",
        "strongest_role",
        "is_team_admin",
        "permissions",
        "member_route",
        "assign_groups",
        "default_permissions",
        "all_permissions",
        "denied",
    ]
    caller: CallerInput
    team: TeamInput
    personal_key: bool
    allow: list[TeamRole]
    org_admins: list[list[str]]
    role: Literal["admin", "user"] | None
    route: str
    access_group_ids: list[str] | None


class Input(TypedDict):
    steps: list[Step]


class ErrorOut(TypedDict):
    status: int | None
    error_type: str | None
    param: str | None
    code: str | None


class StepResult(TypedDict):
    value: Answer
    error: ErrorOut | None
    org_lookups: list[list[str]]


# OrgRoles answered from the case's (user_id, organization_id) pairs, recording each lookup
@dataclass
class CaseOrgRoles:
    admins: list[list[str]]
    lookups: list[list[str]] = field(default_factory=list[list[str]])

    async def is_org_admin(self, user_id: str, organization_id: str) -> bool:
        self.lookups.append([user_id, organization_id])
        return [user_id, organization_id] in self.admins


def run(input_value: Input) -> list[StepResult]:
    results: list[StepResult] = []
    for step in input_value["steps"]:
        roles = CaseOrgRoles(admins=step["org_admins"])
        results.append(asyncio.run(run_step(step, roles)))
    return results


async def run_step(step: Step, roles: CaseOrgRoles) -> StepResult:
    value: Answer = None
    error: ErrorOut | None = None
    try:
        value = await answer(step, build_caller(step["caller"]), build_team(step["team"]), roles)
    except ProxyException as denial:
        error = {
            "status": int(denial.code),
            "error_type": denial.type,
            "param": denial.param,
            "code": denial.code,
        }
    except HTTPException as denial:
        error = {"status": denial.status_code, "error_type": None, "param": None, "code": None}
    except ValueError:
        error = {"status": None, "error_type": None, "param": None, "code": None}
    return {"value": value, "error": error, "org_lookups": roles.lookups}


def build_caller(caller: CallerInput) -> UserAPIKeyAuth:
    role = caller["user_role"]
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=None if role is None else LitellmUserRoles(role),
        team_id=caller["team_id"],
        metadata={"service_account_id": caller["metadata"]["service_account_id"]},
    )


def build_team(team: TeamInput) -> LiteLLM_TeamTableCachedObj:
    members = [
        Member(user_id=member["user_id"], user_email=member["user_email"], role=member["role"])
        for member in team["members_with_roles"]
    ]
    return LiteLLM_TeamTableCachedObj(
        team_id=team["team_id"],
        organization_id=team["organization_id"],
        members_with_roles=members,
        team_member_permissions=team["team_member_permissions"],
    )


async def answer(
    step: Step,
    caller: UserAPIKeyAuth,
    team: LiteLLM_TeamTableCachedObj,
    roles: CaseOrgRoles,
) -> Answer:
    checks = TeamMemberPermissionChecks
    value: Answer = None
    match step["function"]:
        case "allows":
            value = await TeamAccess(org_roles=roles).allows(caller, team, frozenset(step["allow"]))
        case "strongest_role":
            value = await TeamAccess(org_roles=roles).strongest_role(caller, team)
        case "is_team_admin":
            value = is_team_admin(caller, team)
        case "permissions":
            value = sorted(route.value for route in checks.get_permissions_for_team_member(team))
        case "member_route":
            value = checks.does_team_member_have_permissions_for_endpoint(
                team_member_role=step["role"],
                team_table=team,
                route=step["route"],
            )
        case "assign_groups":
            target = None if step["personal_key"] else team
            checks.enforce_member_can_assign_access_groups(caller, target, step["access_group_ids"])
        case "default_permissions":
            value = checks.default_team_member_permissions()
        case "all_permissions":
            value = checks.get_all_available_team_member_permissions()
        case "denied":
            team_access_denied()
    return value
