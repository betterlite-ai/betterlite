from typing import Final, Literal, TypedDict

from litellm.models.team import LiteLLM_TeamTable
from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import ProxyException
from litellm.proxy.auth.resolvers.grants import (
    NotAMember,
    TeamGone,
    UserGone,
    canonical_user_id,
    raise_public,
    user_models,
)
from litellm.proxy.auth.team_grants import TeamGrants, team_grants, team_model_aliases
from pydantic import JsonValue, TypeAdapter, ValidationError
from starlette.exceptions import HTTPException

JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
GRANTS: Final[TypeAdapter[TeamGrants]] = TypeAdapter(TeamGrants)
DETAIL: Final[TypeAdapter[str | dict[str, str]]] = TypeAdapter(str | dict[str, str])


class UserRow(TypedDict):
    user: dict[str, JsonValue] | None
    user_id: str | None


class Denial(TypedDict):
    kind: Literal["user_gone", "team_gone", "not_a_member"]
    user_id: str
    team_id: str


class TeamRow(TypedDict):
    team: dict[str, JsonValue] | None
    membership: dict[str, JsonValue] | None
    user_id: str | None


class Input(TypedDict):
    users: list[UserRow]
    denials: list[Denial]
    teams: list[TeamRow]


class UserView(TypedDict):
    models: list[str] | None
    canonical_user_id: str | None


class DenialView(TypedDict):
    category: Literal["proxy", "http"]
    status: int | None
    error_type: str | None
    param: str | None
    code: str | None
    detail: str | dict[str, str]


class TeamView(TypedDict):
    row: Literal["absent", "invalid", "valid"]
    model_aliases: dict[str, str] | None
    grants: JsonValue


class Answer(TypedDict):
    users: list[UserView]
    denials: list[DenialView]
    teams: list[TeamView]


def user_view(row: UserRow) -> UserView:
    user = None if row["user"] is None else LiteLLM_UserTable.model_validate(row["user"])
    models = None if user is None else list(user_models(user))
    return {"models": models, "canonical_user_id": canonical_user_id(row["user_id"], user)}


def grant_denial(denial: Denial) -> UserGone | TeamGone | NotAMember:
    match denial["kind"]:
        case "user_gone":
            return UserGone(user_id=denial["user_id"])
        case "team_gone":
            return TeamGone(team_id=denial["team_id"])
        case "not_a_member":
            return NotAMember(user_id=denial["user_id"], team_id=denial["team_id"])


def denial_views(denials: list[Denial]) -> list[DenialView]:
    views: list[DenialView] = []
    for denial in denials:
        try:
            raise_public(grant_denial(denial))
        except ProxyException as error:
            views.append(
                {
                    "category": "proxy",
                    "status": None,
                    "error_type": error.type,
                    "param": error.param,
                    "code": error.code,
                    "detail": error.message,
                }
            )
        except HTTPException as error:
            views.append(
                {
                    "category": "http",
                    "status": error.status_code,
                    "error_type": None,
                    "param": None,
                    "code": None,
                    "detail": DETAIL.validate_python(error.detail),
                }
            )
    return views


def team_view(row: TeamRow) -> TeamView:
    membership: LiteLLM_TeamMembership | None = None
    if row["membership"] is not None:
        membership = LiteLLM_TeamMembership.model_validate(row["membership"])
    team: LiteLLM_TeamTable | None = None
    if row["team"] is not None:
        try:
            team = LiteLLM_TeamTable.model_validate(row["team"])
        except ValidationError:
            return {"row": "invalid", "model_aliases": None, "grants": None}
    aliases = team_model_aliases(team)
    granted = team_grants(team, membership, row["user_id"])
    return {
        "row": "absent" if team is None else "valid",
        "model_aliases": None if aliases is None else dict(aliases),
        "grants": JSON_VALUE.validate_json(GRANTS.dump_json(granted)),
    }


def run(input_value: Input) -> Answer:
    return {
        "users": [user_view(row) for row in input_value["users"]],
        "denials": denial_views(input_value["denials"]),
        "teams": [team_view(row) for row in input_value["teams"]],
    }
