from typing import Protocol, runtime_checkable

from litellm.proxy.hooks import PROXY_HOOKS
from pydantic import ConfigDict, TypeAdapter

from .fakes import FixedFailClosed, FixedTime, GroupMap, RecordingCache
from .types import Operations


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self,
        internal_usage_cache: RecordingCache,
        time_provider: FixedTime,
        *,
        model_group_resolver: GroupMap,
        fail_closed_resolver: FixedFailClosed,
    ) -> Operations: ...


def make_limiter(cache: RecordingCache, now: int, groups: dict[str, str]) -> Operations:
    constructor = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(
        cache,
        FixedTime(now),
        model_group_resolver=GroupMap(groups),
        fail_closed_resolver=FixedFailClosed(enabled=False),
    )
