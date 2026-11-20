from collections.abc import Awaitable, Callable, Mapping, Sequence
from typing import Protocol

from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    TagRateLimit,
)
from litellm.proxy.utils import InternalUsageCache
from litellm.types.agents import AgentResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

from .types import DescriptorView, RateLimitView


class TagResolver(Protocol):
    def __call__(self, tag_names: Sequence[str], /) -> Awaitable[Mapping[str, TagRateLimit]]: ...


class Operations(Protocol):
    window_size: int

    def _get_resolved_agent_id(
        self, user_api_key_dict: UserAPIKeyAuth, data: Mapping[str, JsonValue]
    ) -> str | None: ...

    def _get_session_id_from_data(self, data: Mapping[str, JsonValue]) -> str | None: ...

    def _create_agent_rate_limit_descriptors(  # ast-grep-ignore: no-long-names
        self,
        agent_id: str,
        data: Mapping[str, JsonValue],
        policy: AgentResponse | None = None,
    ) -> list[RateLimitDescriptor]: ...

    async def _create_tag_rate_limit_descriptors(  # ast-grep-ignore: no-long-names
        self, data: Mapping[str, JsonValue]
    ) -> tuple[RateLimitDescriptor, ...]: ...

    def _add_tag_per_key_rate_limit_descriptor(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        data: Mapping[str, JsonValue],
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    def _add_mcp_per_key_rate_limit_descriptor(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        mcp_server_name: str | None,
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    def _add_mcp_per_team_rate_limit_descriptor(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        mcp_server_name: str | None,
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    async def tag_descriptors(self, data: Mapping[str, JsonValue]) -> list[DescriptorView]:
        descriptors = await self._create_tag_rate_limit_descriptors(data)
        return [descriptor_view(entry) for entry in descriptors]

    def tag_per_key(
        self, auth: UserAPIKeyAuth, data: Mapping[str, JsonValue]
    ) -> list[DescriptorView]:
        descriptors: list[RateLimitDescriptor] = []
        self._add_tag_per_key_rate_limit_descriptor(auth, data, descriptors)
        return [descriptor_view(entry) for entry in descriptors]

    def mcp_per_key(
        self, auth: UserAPIKeyAuth, mcp_server_name: str | None
    ) -> list[DescriptorView]:
        descriptors: list[RateLimitDescriptor] = []
        self._add_mcp_per_key_rate_limit_descriptor(auth, mcp_server_name, descriptors)
        return [descriptor_view(entry) for entry in descriptors]

    def mcp_per_team(
        self, auth: UserAPIKeyAuth, mcp_server_name: str | None
    ) -> list[DescriptorView]:
        descriptors: list[RateLimitDescriptor] = []
        self._add_mcp_per_team_rate_limit_descriptor(auth, mcp_server_name, descriptors)
        return [descriptor_view(entry) for entry in descriptors]

    def agent_descriptors(
        self,
        agent_id: str,
        data: Mapping[str, JsonValue],
        policy: AgentResponse | None,
    ) -> list[DescriptorView]:
        descriptors = self._create_agent_rate_limit_descriptors(agent_id, data, policy)
        return [descriptor_view(entry) for entry in descriptors]

    def agent_resolution(
        self, auth: UserAPIKeyAuth, data: Mapping[str, JsonValue]
    ) -> str | None:
        return self._get_resolved_agent_id(auth, data)

    def session_resolution(self, data: Mapping[str, JsonValue]) -> str | None:
        return self._get_session_id_from_data(data)


class MapResolver:
    def __init__(self, limits: dict[str, TagRateLimit]) -> None:
        self.limits = limits
        self.calls: list[list[str]] = []

    async def __call__(self, tag_names: Sequence[str], /) -> Mapping[str, TagRateLimit]:
        self.calls.append(list(tag_names))
        return self.limits


def descriptor_view(descriptor: RateLimitDescriptor) -> DescriptorView:
    rate_limit = descriptor["rate_limit"]
    if rate_limit is None:
        return DescriptorView(key=descriptor["key"], value=descriptor["value"], rate_limit=None)
    return DescriptorView(
        key=descriptor["key"],
        value=descriptor["value"],
        rate_limit=RateLimitView(
            requests_per_unit=rate_limit.get("requests_per_unit"),
            tokens_per_unit=rate_limit.get("tokens_per_unit"),
            max_parallel_requests=rate_limit.get("max_parallel_requests"),
            window_size=rate_limit.get("window_size"),
        ),
    )


CONSTRUCTOR: Callable[..., Operations] = TypeAdapter(
    Callable[..., Operations], config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(PROXY_HOOKS["parallel_request_limiter"])


def make_limiter(resolver: TagResolver) -> Operations:
    cache = InternalUsageCache(DualCache())
    return CONSTRUCTOR(cache, tag_rate_limit_resolver=resolver)
