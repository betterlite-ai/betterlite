from collections.abc import Callable
from typing import Protocol, runtime_checkable

from litellm.proxy.hooks import PROXY_HOOKS
from pydantic import ConfigDict, JsonValue, TypeAdapter

from .answers import io_token_answer, scope_targets_answer, success_answer
from .inputs import Input
from .operations import Answer, Operations


class NoRedisCache:
    redis_cache: JsonValue = None


class LimiterCache:
    def __init__(self) -> None:
        self.dual_cache = NoRedisCache()


class GroupMap:
    def __init__(self, groups: dict[str, str]) -> None:
        self.groups = groups

    def __call__(self, model: str) -> str | None:
        return self.groups.get(model)


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self,
        internal_usage_cache: LimiterCache,
        *,
        model_group_resolver: Callable[[str], str | None],
    ) -> Operations: ...


def make_limiter(groups: dict[str, str]) -> Operations:
    constructor = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(LimiterCache(), model_group_resolver=GroupMap(groups))


def run(input_value: Input) -> Answer:
    if input_value["function"] == "io_token_ops":
        return io_token_answer(input_value, make_limiter({}))
    if input_value["function"] == "scope_targets":
        return scope_targets_answer(input_value, make_limiter({}))
    groups = input_value.get("groups")
    return success_answer(input_value, make_limiter({} if groups is None else groups))
