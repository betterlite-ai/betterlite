import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.models.organization_membership import LiteLLM_OrganizationMembershipTable
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import LitellmUserRoles, ProxyException
from litellm.proxy.auth import auth_checks_organization
from pydantic import JsonValue, TypeAdapter

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
# the pinned checks never read a membership's timestamps, so one value fills them
EPOCH: Final = datetime(2026, 1, 1, tzinfo=UTC)
USER_ID: Final = "user-1"


class MembershipInput(TypedDict):
    organization_id: NotRequired[str | None]
    user_role: NotRequired[str | None]


class UserInput(TypedDict):
    user_role: NotRequired[str | None]
    organization_memberships: NotRequired[list[MembershipInput] | None]


class UserCall(TypedDict):
    user: NotRequired[UserInput | None]


class RoleCheckCall(UserCall):
    function: Literal["role_check"]
    route: str
    request: NotRequired[dict[str, JsonValue]]


class OrgInfoCall(UserCall):
    function: Literal["org_info"]


class OrgAdminCall(UserCall):
    function: Literal["org_admin"]
    request: NotRequired[dict[str, JsonValue]]


class TeamContextCall(TypedDict):
    function: Literal["team_context"]
    route: str
    request: NotRequired[dict[str, JsonValue]]
    route_template: NotRequired[str | None]
    team_organization_id: NotRequired[str | None]
    lookup_error: NotRequired[bool]


Call: TypeAlias = RoleCheckCall | OrgInfoCall | OrgAdminCall | TeamContextCall


class Input(TypedDict):
    calls: list[Call]


class Denial(TypedDict):
    type: str
    param: str | None
    code: str


class RoleCheckAnswer(TypedDict):
    denied: Denial | None


class OrganizationInfoAnswer(TypedDict):
    organizations: list[str]
    roles: dict[str, LitellmUserRoles | None]


class OrgAdminAnswer(TypedDict):
    admin: bool


class TeamContextAnswer(TypedDict):
    body: dict[str, JsonValue]
    fetches: list[str]


class LookupFailureError(Exception):
    def __init__(self, team_id: str) -> None:
        super().__init__(f"the scripted team lookup failed for {team_id}")


class ScriptedLookup:
    def __init__(self, *, organization_id: str | None, fail: bool) -> None:
        self.organization_id: str | None = organization_id
        self.fail: bool = fail
        self.fetches: list[str] = []

    async def __call__(self, team_id: str) -> str | None:
        self.fetches.append(team_id)
        if self.fail:
            raise LookupFailureError(team_id)
        return self.organization_id


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [guarded(call) for call in input_value["calls"]]
    return _JSON.validate_python(answers)


def guarded(call: Call) -> JsonValue:
    try:
        return _JSON.validate_python({"ok": True, "value": answer(call)})
    except Exception:  # noqa: BLE001 - a call the pinned code cannot answer reports not-ok  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks_organization.py#L13-L216
        return _JSON.validate_python({"ok": False})


def answer(call: Call) -> JsonValue:
    if call["function"] == "role_check":
        return role_check_answer(call)
    if call["function"] == "org_info":
        return org_info_answer(call)
    if call["function"] == "org_admin":
        return org_admin_answer(call)
    return team_context_answer(call)


def role_check_answer(call: RoleCheckCall) -> JsonValue:
    request_value = call.get("request")
    body: dict[str, JsonValue] = {} if request_value is None else request_value
    try:
        auth_checks_organization.organization_role_based_access_check(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks_organization.py#L13-L17
            request_body=body,
            user_object=user_of(call.get("user")),
            route=call["route"],
        )
    except ProxyException as denied:
        denied_value = RoleCheckAnswer(
            denied=Denial(type=denied.type, param=denied.param, code=denied.code)
        )
        return _JSON.validate_python(denied_value)
    return _JSON.validate_python(RoleCheckAnswer(denied=None))


def org_info_answer(call: OrgInfoCall) -> JsonValue:
    user = user_of(call.get("user"))
    if user is None:
        raise ValueError("the org_info answer needs a user")
    organizations, roles = auth_checks_organization.get_user_organization_info(user)
    return _JSON.validate_python(
        OrganizationInfoAnswer(organizations=organizations, roles=roles)
    )


def org_admin_answer(call: OrgAdminCall) -> JsonValue:
    request_value = call.get("request")
    body: dict[str, JsonValue] = {} if request_value is None else request_value
    admin = auth_checks_organization._user_is_org_admin(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks_organization.py#L136-L139
        request_data=body,
        user_object=user_of(call.get("user")),
    )
    return _JSON.validate_python(OrgAdminAnswer(admin=admin))


def team_context_answer(call: TeamContextCall) -> JsonValue:
    request_value = call.get("request")
    body: dict[str, JsonValue] = {} if request_value is None else request_value
    lookup = ScriptedLookup(
        organization_id=call.get("team_organization_id"),
        fail=bool(call.get("lookup_error")),
    )
    enriched: dict[str, JsonValue] = _BODY.validate_python(asyncio.run(auth_checks_organization.add_team_org_context_to_request_body(route=call["route"], request_body=body, fetch_team_org_id=lookup, route_template=call.get("route_template"))))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks_organization.py#L183-L216
    return _JSON.validate_python(
        TeamContextAnswer(body=enriched, fetches=lookup.fetches)
    )


def user_of(user: UserInput | None) -> LiteLLM_UserTable | None:
    if user is None:
        return None
    membership_rows = user.get("organization_memberships")
    memberships = (
        None
        if membership_rows is None
        else [
            LiteLLM_OrganizationMembershipTable.model_construct(
                user_id=USER_ID,
                organization_id=row.get("organization_id"),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/organization_membership.py#L21
                user_role=row.get("user_role"),
                created_at=EPOCH,
                updated_at=EPOCH,
            )
            for row in membership_rows
        ]
    )
    return LiteLLM_UserTable(
        user_id=USER_ID,
        user_role=user.get("user_role"),
        organization_memberships=memberships,
    )
