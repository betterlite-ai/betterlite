from collections.abc import Callable, Mapping, Sequence
from typing import Protocol, runtime_checkable

from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    TagRateLimit,
    TagRateLimitResolver,
)
from litellm.proxy.utils import InternalUsageCache
from litellm.types.agents import AgentResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

from .types import (
    AuthFields,
    CreatePolicy,
    DescriptorAnswer,
    DescriptorView,
    ServerFields,
    TagLimitRow,
)

AUTH: TypeAdapter[UserAPIKeyAuth] = TypeAdapter(UserAPIKeyAuth)
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)
DESCRIPTOR: TypeAdapter[DescriptorView] = TypeAdapter(DescriptorView)


class GroupMap:
    def __init__(self, groups: dict[str, str]) -> None:
        self.groups = groups

    def __call__(self, model: str) -> str | None:
        return self.groups.get(model)


class TagLimits:
    def __init__(self, limits: dict[str, TagRateLimit]) -> None:
        self.limits = limits

    async def __call__(
        self, tag_names: Sequence[str], /
    ) -> Mapping[str, TagRateLimit]:
        del tag_names
        return self.limits


CreateDescriptors = Callable[
    [UserAPIKeyAuth, dict[str, JsonValue], str | None, str | None, bool, str | None],
    list[RateLimitDescriptor],
]


class Operations(Protocol):
    window_size: int

    def _is_dynamic_rate_limiting_enabled(  # ast-grep-ignore: no-long-names
        self, rpm_limit_type: str | None, tpm_limit_type: str | None
    ) -> bool: ...

    def _should_enforce_rate_limit(
        self, limit_type: str | None, *, model_has_failures: bool
    ) -> bool: ...

    def _get_enforced_limit(
        self,
        limit_value: int | None,
        limit_type: str | None,
        *,
        model_has_failures: bool,
    ) -> int | None: ...

    # a def stops at five parameters, so the six-argument call type is a property value
    @property
    def _create_rate_limit_descriptors(self) -> CreateDescriptors: ...

    async def _build_request_rate_limit_descriptors(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        data: dict[str, JsonValue],
        call_type: str | None,
    ) -> list[RateLimitDescriptor]: ...

    def dynamic_flag(
        self, rpm_limit_type: str | None, tpm_limit_type: str | None
    ) -> bool:
        return self._is_dynamic_rate_limiting_enabled(rpm_limit_type, tpm_limit_type)

    def enforce(self, limit_type: str | None, *, model_has_failures: bool) -> bool:
        return self._should_enforce_rate_limit(
            limit_type, model_has_failures=model_has_failures
        )

    def enforced_limit(
        self,
        limit_value: int | None,
        limit_type: str | None,
        *,
        model_has_failures: bool,
    ) -> int | None:
        return self._get_enforced_limit(
            limit_value, limit_type, model_has_failures=model_has_failures
        )

    def create(
        self,
        auth: UserAPIKeyAuth,
        data: dict[str, JsonValue],
        policy: CreatePolicy,
        call_type: str | None,
    ) -> DescriptorAnswer:
        descriptors = self._create_rate_limit_descriptors(
            auth,
            data,
            policy["rpm_limit_type"],
            policy["tpm_limit_type"],
            policy["model_has_failures"],
            call_type,
        )
        return DescriptorAnswer(
            descriptors=[descriptor_json(entry) for entry in descriptors]
        )

    async def assembled(
        self, auth: UserAPIKeyAuth, data: dict[str, JsonValue], call_type: str | None
    ) -> DescriptorAnswer:
        descriptors = await self._build_request_rate_limit_descriptors(
            auth, data, call_type
        )
        return DescriptorAnswer(
            descriptors=[descriptor_json(entry) for entry in descriptors]
        )


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self,
        internal_usage_cache: InternalUsageCache,
        *,
        tag_rate_limit_resolver: TagRateLimitResolver,
        model_group_resolver: Callable[[str], str | None],
    ) -> Operations: ...


def make_limiter(groups: dict[str, str], tags: TagRateLimitResolver) -> Operations:
    constructor = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(
        InternalUsageCache(DualCache()),
        tag_rate_limit_resolver=tags,
        model_group_resolver=GroupMap(groups),
    )


def make_auth(fields: AuthFields, server: ServerFields) -> UserAPIKeyAuth:
    auth = AUTH.validate_python(fields)
    managed = server.get("managed_agent_policy")
    invoked = server.get("invoked_agent_policy")
    auth.managed_agent_policy = None if managed is None else AGENT.validate_python(managed)
    auth.invoked_agent_policy = None if invoked is None else AGENT.validate_python(invoked)
    auth.invoked_agent_id = server.get("invoked_agent_id")
    auth.mcp_source_team_rpm_limits = server.get("source_team_rpm_limits")
    return auth


def descriptor_json(descriptor: RateLimitDescriptor) -> DescriptorView:
    rate_limit = descriptor["rate_limit"]
    if rate_limit is None:
        return DESCRIPTOR.validate_python(
            {"key": descriptor["key"], "value": descriptor["value"], "rate_limit": None}
        )
    filled = {
        "requests_per_unit": rate_limit.get("requests_per_unit"),
        "tokens_per_unit": rate_limit.get("tokens_per_unit"),
        "max_parallel_requests": rate_limit.get("max_parallel_requests"),
        "window_size": rate_limit.get("window_size"),
    }
    return DESCRIPTOR.validate_python(
        {"key": descriptor["key"], "value": descriptor["value"], "rate_limit": filled}
    )


def tag_limits_of(rows: list[TagLimitRow]) -> dict[str, TagRateLimit]:
    return {
        row["tag_name"]: TagRateLimit(
            rpm_limit=row["rpm_limit"], tpm_limit=row["tpm_limit"]
        )
        for row in rows
    }
