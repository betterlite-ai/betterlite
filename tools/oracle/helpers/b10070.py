import asyncio
import sys
from types import ModuleType
from typing import Literal, TypeAlias, TypedDict

from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._experimental.mcp_server.ui_session_utils import (
    is_ui_session_credential,
    resolve_ui_session_team_ids,
)
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE = "litellm.proxy.proxy_server"
CHECKS_MODULE = "litellm.proxy.auth.auth_checks"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class AuthFields(TypedDict):
    team_id: str | None
    user_id: str | None
    requires_fresh_policy: bool


class SessionCall(TypedDict):
    operation: Literal["session"]
    auth: AuthFields


class TeamsAnswer(TypedDict):
    kind: Literal["teams"]
    teams: list[str]


class NoUserAnswer(TypedDict):
    kind: Literal["no_user"]


class FailureAnswer(TypedDict):
    kind: Literal["failure"]


LookupAnswer: TypeAlias = TeamsAnswer | NoUserAnswer | FailureAnswer


class TeamIdsCall(TypedDict):
    operation: Literal["team_ids"]
    auth: AuthFields
    database: bool
    lookup: LookupAnswer


Input: TypeAlias = SessionCall | TeamIdsCall


class SessionAnswer(TypedDict):
    session: bool


class TeamIdsAnswer(TypedDict):
    teams: list[str]
    lookup_ran: bool
    fresh: bool | None


class CaseDatabase:
    pass


class ProxyModule(ModuleType):
    def __init__(self, database: bool) -> None:
        super().__init__(PROXY_MODULE)
        self.prisma_client = CaseDatabase() if database else None
        self.proxy_logging_obj = None
        self.user_api_key_cache = None


class ChecksModule(ModuleType):
    def __init__(self, answer: LookupAnswer) -> None:
        super().__init__(CHECKS_MODULE)
        self.answer = answer
        self.ran = False
        self.check_db_only: bool | None = None

    async def get_user_object(
        self,
        user_id: str,
        *,
        check_db_only: bool,
        **_lookup: bool | CaseDatabase | None,
    ) -> LiteLLM_UserTable | None:
        self.ran = True
        self.check_db_only = check_db_only
        answer = self.answer
        if answer["kind"] == "failure":
            raise ValueError("the case user lookup failed")
        if answer["kind"] == "no_user":
            return None
        return LiteLLM_UserTable(user_id=user_id, teams=answer["teams"])


def install(database: bool, answer: LookupAnswer) -> ChecksModule:
    sys.modules[PROXY_MODULE] = ProxyModule(database)
    checks = ChecksModule(answer)
    sys.modules[CHECKS_MODULE] = checks
    return checks


def auth_of(fields: AuthFields) -> UserAPIKeyAuth:
    auth = UserAPIKeyAuth(team_id=fields["team_id"], user_id=fields["user_id"])
    # LiteLLM strips requires_fresh_policy from constructor input and sets it after construction
    auth.requires_fresh_policy = fields["requires_fresh_policy"]
    return auth


def session(call: SessionCall) -> JsonValue:
    answer = SessionAnswer(session=is_ui_session_credential(auth_of(call["auth"])))
    return JSON.validate_python(answer)


def team_ids(call: TeamIdsCall) -> JsonValue:
    auth = auth_of(call["auth"])
    checks = install(call["database"], call["lookup"])
    teams: list[str] = asyncio.run(resolve_ui_session_team_ids(auth))
    answer = TeamIdsAnswer(teams=teams, lookup_ran=checks.ran, fresh=checks.check_db_only)
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "session":
        return session(input_value)
    return team_ids(input_value)
