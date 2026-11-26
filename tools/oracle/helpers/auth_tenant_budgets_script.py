import logging
import sys
from collections.abc import Sequence
from datetime import datetime
from types import ModuleType
from typing import ClassVar, Final

from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.tag import LiteLLM_TagTable
from litellm.proxy._types import CallInfo
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.types.proxy.model_access_group_budget import ModelAccessGroupBudget
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


# the collaborators one call hands to the production functions, and the lookups they answer
class Script:
    organization: ClassVar[LiteLLM_OrganizationTable | None] = None
    org_raises: ClassVar[bool] = False
    tag_rows: ClassVar[dict[str, LiteLLM_TagTable]] = {}
    group_rows: ClassVar[dict[str, ModelAccessGroupBudget]] = {}


class Recorder(ProxyLogging):
    def __init__(self) -> None:
        self.alerts: Final[list[JsonValue]] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        self.alerts.append({"type": type, "info": JSON.validate_json(user_info.model_dump_json())})


class SoftLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.messages: Final[list[str]] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName == "_project_soft_budget_check":
            self.messages.append(record.getMessage())


class CounterReads:
    def __init__(self, spends: list[float]) -> None:
        self.spends: Final = spends
        self.reads: Final[list[JsonValue]] = []

    async def __call__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2797-L2806
        self,
        counter_key: str,
        fallback_spend: float,
        max_budget: float | None = None,
        window_entity_type: str | None = None,
        window_entity_id: str | None = None,
        window_duration: str | None = None,
        window_start: datetime | None = None,
        *,
        fallback_authoritative: bool = False,
    ) -> float:
        del window_entity_type, window_entity_id, window_duration, window_start
        self.reads.append(
            {
                "counter_key": counter_key,
                "fallback_spend": fallback_spend,
                "max_budget": max_budget,
                "fallback_authoritative": fallback_authoritative,
            }
        )
        if len(self.reads) > len(self.spends):
            raise RuntimeError("the scripted spend ran out")
        return self.spends[len(self.reads) - 1]


# only presence is tested: the batch lookups are patched in the entry points
class SeededDatabase(PrismaClient):
    def __init__(self) -> None:
        pass


class ProxyServer(ModuleType):
    get_current_spend: CounterReads


async def scripted_org_object(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4107-L4130
    org_id: str,
    prisma_client: PrismaClient | None,
    user_api_key_cache: UserApiKeyCache,
    proxy_logging_obj: ProxyLogging | None = None,
    include_budget_table: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4107-L4130
) -> LiteLLM_OrganizationTable | None:
    del prisma_client, user_api_key_cache, proxy_logging_obj, include_budget_table
    if Script.org_raises:
        raise RuntimeError(f"the organization lookup failed for {org_id}")
    organization = Script.organization
    if organization is None or organization.organization_id != org_id:
        return None
    return organization


async def scripted_tag_objects(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2247-L2262
    tag_names: Sequence[str],
    prisma_client: PrismaClient | None,
    user_api_key_cache: UserApiKeyCache,
    parent_otel_span: JsonValue | None = None,
    proxy_logging_obj: ProxyLogging | None = None,
) -> dict[str, LiteLLM_TagTable]:
    del prisma_client, user_api_key_cache, parent_otel_span, proxy_logging_obj
    return {name: row for name, row in Script.tag_rows.items() if name in tag_names}


async def scripted_group_budgets(
    access_group_names: Sequence[str],
    prisma_client: PrismaClient | None,
    user_api_key_cache: UserApiKeyCache,
) -> dict[str, ModelAccessGroupBudget]:
    del prisma_client, user_api_key_cache
    return {name: row for name, row in Script.group_rows.items() if name in access_group_names}


def install(reader: CounterReads) -> None:
    proxy = ProxyServer("litellm.proxy.proxy_server")
    proxy.get_current_spend = reader
    sys.modules["litellm.proxy.proxy_server"] = proxy
