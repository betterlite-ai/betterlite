import asyncio
import sys
from collections.abc import Generator, Mapping
from contextlib import contextmanager
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

from .inputs import LookupCall, RecordLookup, budget_table, parse_optional

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

    def record(self, name: str, args: Mapping[str, JsonValue]) -> None:
        entry: dict[str, JsonValue] = {"name": name, "args": dict(args)}
        self.entries.append(entry)

    def record_fields(self) -> RecordLookup | None:
        lookup = self.lookup
        if lookup["kind"] == "missing":
            return None
        return lookup


class TeamLookup:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted

    async def __call__(
        self,
        team_id: str,
        *_args: LookupArgument,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_TeamTableCachedObj:
        await asyncio.sleep(0)
        self._scripted.record("team_info", {"team_id": team_id})
        record = self._scripted.record_fields()
        if record is None:
            raise TeamNotFoundError(team_id=team_id)
        return LiteLLM_TeamTableCachedObj(
            team_id=team_id,
            max_budget=record["max_budget"],
            budget_reset_at=parse_optional(record["budget_reset_at"]),
        )


class OrgLookup:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted

    async def __call__(
        self,
        org_id: str,
        *_args: LookupArgument,
        include_budget_table: bool = False,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_OrganizationTable | None:
        await asyncio.sleep(0)
        self._scripted.record(
            "org_info",
            {"org_id": org_id, "include_budget_table": include_budget_table},
        )
        record = self._scripted.record_fields()
        if record is None:
            return None
        spend = record["spend"]
        return LiteLLM_OrganizationTable(
            organization_id=org_id,
            organization_alias=record["organization_alias"],
            spend=0.0 if spend is None else spend,
            budget_id="scripted-budget",
            created_by="oracle",
            updated_by="oracle",
            litellm_budget_table=budget_table(record["budget"]),
        )


class KeyLookup:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted

    async def __call__(
        self,
        hashed_token: str,
        *_args: LookupArgument,
        check_cache_only: bool | None = None,
        **_kwargs: LookupArgument,
    ) -> UserAPIKeyAuth:
        await asyncio.sleep(0)
        self._scripted.record(
            "key_info",
            {"hashed_token": hashed_token, "check_cache_only": check_cache_only},
        )
        record = self._scripted.record_fields()
        if record is None:
            raise RuntimeError("the key is not in the cache")
        return UserAPIKeyAuth(
            token=hashed_token,
            budget_reset_at=parse_optional(record["budget_reset_at"]),
        )


class UserLookup:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted

    async def __call__(
        self,
        user_id: str | None,
        *_args: LookupArgument,
        user_id_upsert: bool | None = None,
        check_db_only: bool | None = None,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_UserTable | None:
        await asyncio.sleep(0)
        if user_id is None:
            return None
        self._scripted.record(
            "user_info",
            {
                "user_id": user_id,
                "user_id_upsert": user_id_upsert,
                "check_db_only": check_db_only,
            },
        )
        record = self._scripted.record_fields()
        if record is None:
            return None
        return LiteLLM_UserTable(
            user_id=user_id,
            max_budget=record["max_budget"],
            budget_reset_at=parse_optional(record["budget_reset_at"]),
            user_email=record["user_email"],
            user_alias=record["user_alias"],
        )


class EndUserCache:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted

    async def async_get_cache(
        self,
        key: str,
        *_args: LookupArgument,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_EndUserTable | None:
        await asyncio.sleep(0)
        self._scripted.record("cached_customer", {"cache_key": key})
        record = self._scripted.record_fields()
        if record is None:
            return None
        spend = record["spend"]
        return LiteLLM_EndUserTable(
            user_id=key.removeprefix("end_user_id:"),
            blocked=False,
            spend=0.0 if spend is None else spend,
            litellm_budget_table=budget_table(record["budget"]),
        )


class FakeBudgetTable:
    def __init__(self, scripted: ScriptedLookups) -> None:
        self._scripted = scripted

    async def find_unique(self, *, where: Mapping[str, str]) -> LiteLLM_BudgetTableFull | None:
        await asyncio.sleep(0)
        budget_id = where["budget_id"]
        self._scripted.record("default_customer_budget", {"budget_id": budget_id})
        record = self._scripted.record_fields()
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


@contextmanager
def patched(scripted: ScriptedLookups) -> Generator[None]:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = FakePrismaClient(scripted) if scripted.database else None
    proxy_server.user_api_key_cache = EndUserCache(scripted)
    saved_module = sys.modules.get(PROXY_MODULE)
    saved_team = auth_checks.get_team_object
    saved_org = auth_checks.get_org_object
    saved_key = auth_checks.get_key_object
    saved_user = auth_checks.get_user_object
    sys.modules[PROXY_MODULE] = proxy_server
    auth_checks.get_team_object = TeamLookup(scripted)
    auth_checks.get_org_object = OrgLookup(scripted)
    auth_checks.get_key_object = KeyLookup(scripted)
    auth_checks.get_user_object = UserLookup(scripted)
    try:
        yield
    finally:
        auth_checks.get_team_object = saved_team
        auth_checks.get_org_object = saved_org
        auth_checks.get_key_object = saved_key
        auth_checks.get_user_object = saved_user
        if saved_module is None:
            del sys.modules[PROXY_MODULE]
        else:
            sys.modules[PROXY_MODULE] = saved_module
