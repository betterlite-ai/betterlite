import asyncio

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.tag import LiteLLM_TagTable
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints import agent_registry
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache, tag_cache_key
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    TagRateLimit,
    resolve_tag_rate_limits_from_db,
)
from litellm.proxy.utils import PrismaClient
from litellm.types.agents import AgentResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .limiter import MapResolver, Operations, make_limiter
from .types import (
    AgentAnswer,
    AgentCall,
    AgentResolutionCall,
    DescriptorsAnswer,
    LimitView,
    McpPerKeyCall,
    McpPerTeamCall,
    SessionAnswer,
    SessionCall,
    TagBudgetFixture,
    TagCreateAnswer,
    TagCreateCall,
    TagDescriptorCall,
    TagPerKeyCall,
    TagResolutionAnswer,
    TagResolutionCall,
)

MANAGEMENT_TTL_SECONDS: float = 60.0

AUTH: TypeAdapter[UserAPIKeyAuth] = TypeAdapter(UserAPIKeyAuth)
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)
TAG: TypeAdapter[LiteLLM_TagTable] = TypeAdapter(LiteLLM_TagTable)


class TagCache(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.tags: dict[str, LiteLLM_TagTable] = {}

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> LiteLLM_TagTable | None:
        del kwargs
        return self.tags.get(key)


class FakeTagRows:
    def __init__(self, tags: list[LiteLLM_TagTable]) -> None:
        self.tags = tags

    async def find_many(
        self,
        where: dict[str, dict[str, list[str]]] | None = None,
        include: dict[str, bool] | None = None,
        take: int | None = None,
    ) -> list[LiteLLM_TagTable]:
        del include
        if take is not None:
            return self.tags[:take]
        if where is None:
            return self.tags
        names = where["tag_name"]["in"]
        return [tag for tag in self.tags if tag.tag_name in names]


class FakeDb(PrismaWrapper):
    def __init__(self, rows: FakeTagRows) -> None:
        self.litellm_tagtable = rows


class FakePrismaClient(PrismaClient):
    def __init__(self, rows: FakeTagRows) -> None:
        self.db = FakeDb(rows)


def tag_table(entry: TagBudgetFixture) -> LiteLLM_TagTable:
    budget = entry["budget"]
    fields: dict[str, JsonValue] = {
        "tag_name": entry["tag_name"],
        "models": [],
        "spend": 0.0,
    }
    if budget is not None:
        fields["litellm_budget_table"] = {
            "rpm_limit": budget["rpm_limit"],
            "tpm_limit": budget["tpm_limit"],
        }
    return TAG.validate_python(fields)


def tag_cache(tags: list[LiteLLM_TagTable]) -> UserApiKeyCache:
    memory = TagCache()
    for tag in tags:
        memory.tags[tag_cache_key(tag.tag_name)] = tag
    return UserApiKeyCache(
        in_memory_cache=memory, default_in_memory_ttl=MANAGEMENT_TTL_SECONDS
    )


async def tag_resolution_answer(call: TagResolutionCall) -> TagResolutionAnswer:
    proxy_server.user_api_key_cache = tag_cache(
        [tag_table(entry) for entry in call["cached"]]
    )
    if call["database"]:
        rows = FakeTagRows([tag_table(entry) for entry in call["rows"]])
        proxy_server.prisma_client = FakePrismaClient(rows)
    else:
        proxy_server.prisma_client = None
    limits = await resolve_tag_rate_limits_from_db(call["tag_names"])
    return TagResolutionAnswer(
        limits={
            name: LimitView(rpm_limit=limit.rpm_limit, tpm_limit=limit.tpm_limit)
            for name, limit in limits.items()
        }
    )


def tag_descriptor_answer(call: TagDescriptorCall) -> DescriptorsAnswer:
    limit = TagRateLimit(rpm_limit=call["rpm_limit"], tpm_limit=call["tpm_limit"])
    limiter = make_limiter(MapResolver({call["tag"]: limit}))
    limiter.window_size = call["window_size"]
    descriptors = asyncio.run(Operations.tag_descriptors(limiter, {"tags": [call["tag"]]}))
    return DescriptorsAnswer(descriptors=descriptors)


def tag_create_answer(call: TagCreateCall) -> TagCreateAnswer:
    limits = {
        row["tag_name"]: TagRateLimit(rpm_limit=row["rpm_limit"], tpm_limit=row["tpm_limit"])
        for row in call["tag_limits"]
    }
    resolver = MapResolver(limits)
    limiter = make_limiter(resolver)
    limiter.window_size = call["window_size"]
    descriptors = asyncio.run(Operations.tag_descriptors(limiter, call["data"]))
    return TagCreateAnswer(descriptors=descriptors, resolver_calls=resolver.calls)


def tag_per_key_answer(call: TagPerKeyCall) -> DescriptorsAnswer:
    limiter = make_limiter(MapResolver({}))
    limiter.window_size = call["window_size"]
    auth = AUTH.validate_python(call["auth"])
    return DescriptorsAnswer(descriptors=Operations.tag_per_key(limiter, auth, call["data"]))


def mcp_per_key_answer(call: McpPerKeyCall) -> DescriptorsAnswer:
    limiter = make_limiter(MapResolver({}))
    limiter.window_size = call["window_size"]
    auth = AUTH.validate_python(call["auth"])
    return DescriptorsAnswer(
        descriptors=Operations.mcp_per_key(limiter, auth, call["mcp_server_name"])
    )


def mcp_per_team_answer(call: McpPerTeamCall) -> DescriptorsAnswer:
    limiter = make_limiter(MapResolver({}))
    limiter.window_size = call["window_size"]
    auth = AUTH.validate_python(call["auth"])
    auth.mcp_source_team_rpm_limits = call["source_team_rpm_limits"]
    return DescriptorsAnswer(
        descriptors=Operations.mcp_per_team(limiter, auth, call["mcp_server_name"])
    )


def agent_answer(call: AgentCall) -> DescriptorsAnswer:
    agent_registry.global_agent_registry.agent_list = [
        AGENT.validate_python(entry) for entry in call["registry"]
    ]
    limiter = make_limiter(MapResolver({}))
    limiter.window_size = call["window_size"]
    policy = None if call["policy"] is None else AGENT.validate_python(call["policy"])
    descriptors = Operations.agent_descriptors(limiter, call["agent_id"], call["data"], policy)
    return DescriptorsAnswer(descriptors=descriptors)


def agent_resolution_answer(call: AgentResolutionCall) -> AgentAnswer:
    limiter = make_limiter(MapResolver({}))
    auth = AUTH.validate_python({"agent_id": call["key_agent_id"]})
    return AgentAnswer(agent_id=Operations.agent_resolution(limiter, auth, call["data"]))


def session_answer(call: SessionCall) -> SessionAnswer:
    limiter = make_limiter(MapResolver({}))
    return SessionAnswer(session_id=Operations.session_resolution(limiter, call["data"]))
