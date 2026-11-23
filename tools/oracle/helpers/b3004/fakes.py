from collections.abc import Mapping, Sequence
from typing import Final

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy import proxy_server
from litellm.proxy.spend_tracking.spend_counter_batch import PendingSpendIncrement
from litellm.router import Router
from pydantic import JsonValue
from typing_extensions import override

# captured before the case scripts it, so the recorder still applies the real increments
REAL_PIPELINE: Final = proxy_server.run_spend_counter_pipeline


class ScriptedMemory(InMemoryCache):
    def __init__(self, values: dict[str, float], failures: frozenset[str]) -> None:
        super().__init__()
        self.values = dict(values)
        self.failures = failures
        self.deleted: list[str] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> float | None:
        del kwargs
        if key in self.failures:
            raise ValueError(f"the scripted spend counter {key} is unavailable")
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: float, **kwargs: float | None) -> None:
        del kwargs
        self.values[key] = value

    @override
    def delete_cache(self, key: str) -> None:
        self.deleted.append(key)
        self.values.pop(key, None)


class ScriptedSources(InMemoryCache):
    # the source cache holds the auth row's JSON, whose "spend" _get_source_cache_base_spend reads
    def __init__(self, sources: dict[str, float]) -> None:
        super().__init__()
        self.entries: dict[str, dict[str, float]] = {
            key: {"spend": amount} for key, amount in sources.items()
        }

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> dict[str, float] | None:
        del kwargs
        return self.entries.get(key)


class Pipeline:
    def __init__(self, *, fail: bool) -> None:
        self.fail = fail
        self.calls: list[JsonValue] = []

    async def __call__(self, pending: Sequence[PendingSpendIncrement]) -> tuple[float | None, ...]:
        if not pending:
            return ()
        recorded: list[JsonValue] = []
        for item in pending:
            pair: list[JsonValue] = [item.counter_key, item.increment]
            recorded.append(pair)
        self.calls.append(recorded)
        if self.fail:
            raise RuntimeError("the scripted spend counter pipeline failed")
        return await REAL_PIPELINE(pending=pending)


class MaxCost:
    def __init__(self, cost: float) -> None:
        self.cost = cost

    def __call__(
        self,
        request_body: Mapping[str, JsonValue],
        route: str,
        llm_router: Router | None,
        input_token_counts: Mapping[str, int] | None = None,
    ) -> float | None:
        del request_body, route, llm_router, input_token_counts
        return self.cost


class InputCost:
    def __call__(
        self,
        request_body: Mapping[str, JsonValue],
        route: str,
        llm_router: Router | None,
        input_token_counts: Mapping[str, int] | None = None,
    ) -> float | None:
        del request_body, route, llm_router, input_token_counts
        return 0.0


class InputTokens:
    async def __call__(
        self,
        request_body: Mapping[str, JsonValue],
        route: str,
        llm_router: Router | None,
        raw_body: bytes | None = None,
    ) -> Mapping[str, int]:
        del request_body, route, llm_router, raw_body
        return {}
