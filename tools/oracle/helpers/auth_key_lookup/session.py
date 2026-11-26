import httpx
from litellm.proxy._types import (
    LiteLLM_TeamMembership,
    LiteLLM_TeamTableCachedObj,
    LiteLLM_UserTable,
    ProxyException,
    UserAPIKeyAuth,
)
from litellm.proxy.auth import user_api_key_auth
from litellm.proxy.auth.auth_checks import TeamNotFoundError
from litellm.proxy.utils import ProxyLogging
from litellm.types.proxy.auth.auth_checks import UserNotFoundError
from pydantic import JsonValue
from starlette.exceptions import HTTPException

from .inputs import JSON, Call, Outcome
from .keys import CacheScript, KeyCache, LookupArgument, ReadScript, ScriptedClient
from .views import auth_fields


class SessionScript:
    def __init__(
        self,
        user: LiteLLM_UserTable | None,
        outcome: Outcome,
        team: LiteLLM_TeamTableCachedObj | None,
        membership: LiteLLM_TeamMembership | None,
    ) -> None:
        self.user = user
        self.outcome = outcome
        self.team = team
        self.membership = membership


class ScriptedUser:
    def __init__(self, script: SessionScript) -> None:
        self.script = script

    async def __call__(
        self, user_id: str | None = None, *_args: LookupArgument, **_kwargs: LookupArgument
    ) -> LiteLLM_UserTable | None:
        outcome = self.script.outcome
        if outcome == "gone":
            raise UserNotFoundError(str(user_id))
        if outcome == "outage":
            raise httpx.ConnectError("connection closed")
        return self.script.user


class ScriptedTeam:
    def __init__(self, script: SessionScript) -> None:
        self.script = script

    async def __call__(
        self, team_id: str, *_args: LookupArgument, **_kwargs: LookupArgument
    ) -> LiteLLM_TeamTableCachedObj:
        team = self.script.team
        if team is None:
            raise TeamNotFoundError(team_id)
        return team


class ScriptedMembership:
    def __init__(self, script: SessionScript) -> None:
        self.script = script

    async def __call__(
        self, user_id: str, team_id: str, *_args: LookupArgument, **_kwargs: LookupArgument
    ) -> LiteLLM_TeamMembership | None:
        del user_id, team_id
        return self.script.membership


def session_script(call: Call) -> SessionScript:
    user_row = call.get("user")
    team_row = call.get("team")
    membership_row = call.get("membership")
    user: LiteLLM_UserTable | None = (
        None if user_row is None else LiteLLM_UserTable.model_validate(user_row)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
    )
    team: LiteLLM_TeamTableCachedObj | None = (
        None if team_row is None else LiteLLM_TeamTableCachedObj.model_validate(team_row)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
    )
    membership = (
        None if membership_row is None else LiteLLM_TeamMembership.model_validate(membership_row)
    )
    outcome = call.get("user_outcome")
    return SessionScript(user, "missing" if outcome is None else outcome, team, membership)


async def session(call: Call) -> JsonValue:
    script = session_script(call)
    auth_row = call.get("auth")
    if auth_row is None:
        raise ValueError("a session call needs the minted auth")
    auth: UserAPIKeyAuth = UserAPIKeyAuth.model_validate(auth_row)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
    client = ScriptedClient(ReadScript([]))
    cache = KeyCache(CacheScript(None))
    logging_obj = ProxyLogging(cache)
    outcome: JsonValue = "ok"
    denial: JsonValue = None
    fields: JsonValue = None
    user_api_key_auth.get_user_object = ScriptedUser(script)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
    user_api_key_auth.get_team_object = ScriptedTeam(script)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
    user_api_key_auth.get_team_membership = ScriptedMembership(script)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
    try:
        refreshed: UserAPIKeyAuth = await user_api_key_auth._refresh_session_token_grants(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
            auth, client, cache, None, logging_obj
        )
        fields = auth_fields(refreshed)
    except TeamNotFoundError:
        outcome = "denied"
        denial = "team_gone"
    except ProxyException:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
        outcome = "denied"
        denial = "user_gone"
    except HTTPException:
        outcome = "denied"
        denial = "not_a_member"
    record: dict[str, JsonValue] = {"outcome": outcome, "denial": denial, "fields": fields}
    return JSON.validate_python(record)
