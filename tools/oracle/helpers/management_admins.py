from typing import Final, Literal, Required, TypeAlias, TypedDict

from litellm.models.team import LiteLLM_TeamTable
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.management.teams.access import is_team_admin
from litellm.proxy.management_endpoints import common_utils
from pydantic import TypeAdapter


class MembershipRow(TypedDict, total=False):
    user_id: Required[str]
    organization_id: Required[str]
    user_role: str | None
    created_at: Required[str]
    updated_at: Required[str]


class UserRow(TypedDict, total=False):
    user_id: Required[str]
    teams: list[str]
    organization_memberships: list[MembershipRow] | None


class MemberRow(TypedDict, total=False):
    user_id: str | None
    user_email: str | None
    role: Required[Literal["admin", "user"]]


class TeamRow(TypedDict, total=False):
    team_id: Required[str]
    members_with_roles: list[MemberRow]


class OrgCase(TypedDict):
    function: Literal["org_admin_can_invite_user"]
    admin: UserRow
    target: UserRow


class RosterCase(TypedDict):
    function: Literal["is_team_admin"]
    caller_user_id: str | None
    team: TeamRow


Case: TypeAlias = OrgCase | RosterCase

USER_ROWS: Final[TypeAdapter[LiteLLM_UserTable]] = TypeAdapter(LiteLLM_UserTable)
TEAM_ROWS: Final[TypeAdapter[LiteLLM_TeamTable]] = TypeAdapter(LiteLLM_TeamTable)


class Input(TypedDict):
    cases: list[Case]


def run(input_value: Input) -> list[bool]:
    outcomes: list[bool] = []
    for case in input_value["cases"]:
        if case["function"] == "org_admin_can_invite_user":
            admin = USER_ROWS.validate_python(case["admin"])
            target = USER_ROWS.validate_python(case["target"])
            outcomes.append(common_utils._org_admin_can_invite_user(admin, target))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/common_utils.py#L290
            continue
        caller = UserAPIKeyAuth(user_id=case["caller_user_id"])
        team = TEAM_ROWS.validate_python(case["team"])
        outcomes.append(is_team_admin(user_api_key_dict=caller, team_obj=team))
    return outcomes
