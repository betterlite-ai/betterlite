import sys
from collections.abc import Mapping
from types import ModuleType
from typing import Final, TypeAlias

from litellm.models.budget import LiteLLM_BudgetTableFull
from litellm.models.end_user import LiteLLM_EndUserTable
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.proxy.auth.auth_checks import TeamNotFoundError
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from opentelemetry.trace import Span
from pydantic import JsonValue

from .inputs import (
    LookupCall,
    RecordLookup,
    budget_table,
    end_user_object,
    key_object,
    org_object,
    team_object,
    user_object,
)

PROXY_MODULE: Final = "litellm.proxy.proxy_server"

# every argument type the four auth lookups take, so the fakes accept their keyword calls
LookupArgument: TypeAlias = (
    str | bool | None | Span | PrismaClient | UserApiKeyCache | ProxyLogging
)


class ScriptedLookups:
    def __init__(self, lookup: LookupCall, *, database: bool) -> None:
        self.lookup = lookup
        self.database = database
        self.entries: list[dict[str, JsonValue]] = []

    def answer(self, name: str, args: Mapping[str, JsonValue]) -> RecordLookup | None:
        entry: dict[str, JsonValue] = {"name": name, "args": dict(args)}
        self.entries.append(entry)
        lookup = self.lookup
        if lookup["kind"] == "missing":
            return None
        return lookup


class ScriptedCall:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted


class TeamLookup(ScriptedCall):
    async def __call__(
        self, team_id: str, *_args: LookupArgument, **_kwargs: LookupArgument
    ) -> LiteLLM_TeamTableCachedObj:
        record = self._scripted.answer("team_info", {"team_id": team_id})
        if record is None:
            raise TeamNotFoundError(team_id=team_id)
        return team_object(team_id, record)


class OrgLookup(ScriptedCall):
    async def __call__(
        self, org_id: str, *_args: LookupArgument, include_budget_table: bool = False,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_OrganizationTable | None:
        record = self._scripted.answer(
            "org_info", {"org_id": org_id, "include_budget_table": include_budget_table}
        )
        return None if record is None else org_object(org_id, record)


class KeyLookup(ScriptedCall):
    async def __call__(
        self, hashed_token: str, *_args: LookupArgument, check_cache_only: bool | None = None,
        **_kwargs: LookupArgument,
    ) -> UserAPIKeyAuth:
        record = self._scripted.answer(
            "key_info", {"hashed_token": hashed_token, "check_cache_only": check_cache_only}
        )
        if record is None:
            raise RuntimeError("the key is not in the cache")
        return key_object(hashed_token, record)


class UserLookup(ScriptedCall):
    async def __call__(
        self, user_id: str | None, *_args: LookupArgument, user_id_upsert: bool | None = None,
        check_db_only: bool | None = None, **_kwargs: LookupArgument,
    ) -> LiteLLM_UserTable | None:
        if user_id is None:
            return None
        record = self._scripted.answer(
            "user_info",
            {
                "user_id": user_id,
                "user_id_upsert": user_id_upsert,
                "check_db_only": check_db_only,
            },
        )
        if record is None:
            return None
        return user_object(user_id, record)


class EndUserCache(ScriptedCall):
    async def async_get_cache(
        self, key: str, *_args: LookupArgument, **_kwargs: LookupArgument
    ) -> LiteLLM_EndUserTable | None:
        record = self._scripted.answer("cached_customer", {"cache_key": key})
        if record is None:
            return None
        return end_user_object(key, record)


class FakeBudgetTable(ScriptedCall):
    async def find_unique(self, *, where: Mapping[str, str]) -> LiteLLM_BudgetTableFull | None:
        record = self._scripted.answer(
            "default_customer_budget", {"budget_id": where["budget_id"]}
        )
        if record is None:
            return None
        return budget_table(record["budget"])


class FakeDb:
    def __init__(self, table: FakeBudgetTable) -> None:
        self.litellm_budgettable = table


class FakePrismaClient(PrismaClient):
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._budget_db = FakeDb(FakeBudgetTable(scripted))

    def __getattr__(self, name: str) -> FakeDb:
        if name == "db":
            return self._budget_db
        raise AttributeError(name)


class ProxyServer(ModuleType):
    prisma_client: FakePrismaClient | None = None
    user_api_key_cache: EndUserCache | None = None


def install_lookups(scripted: ScriptedLookups) -> None:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = FakePrismaClient(scripted) if scripted.database else None
    proxy_server.user_api_key_cache = EndUserCache(scripted)
    sys.modules[PROXY_MODULE] = proxy_server
    auth_checks.get_team_object = TeamLookup(scripted)
    auth_checks.get_org_object = OrgLookup(scripted)
    auth_checks.get_key_object = KeyLookup(scripted)
    auth_checks.get_user_object = UserLookup(scripted)
