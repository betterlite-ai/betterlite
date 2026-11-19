from collections.abc import Awaitable
from typing import Final, Literal, Protocol, Required, TypedDict, cast

import litellm
from litellm import utils as litellm_utils
from litellm.caching.caching import Cache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.types.caching import LiteLLMCacheType
from pydantic import JsonValue
from pydantic_core import to_jsonable_python
from typing_extensions import override


class StepSpec(TypedDict, total=False):
    kind: Required[Literal["response", "error"]]
    value: JsonValue
    error_class: str
    message: str


class BudgetSpec(TypedDict, total=False):
    max_budget: float
    current_cost: float


class RetrySpec(TypedDict, total=False):
    num_retries: int | None
    num_retries_per_request: int | None


class CacheSpec(TypedDict, total=False):
    hit: JsonValue


class ObserveSpec(TypedDict, total=False):
    values: list[str]
    present: list[str]


class Input(TypedDict, total=False):
    mode: Literal["sync", "async"]
    request: Required[dict[str, JsonValue]]
    outcomes: list[StepSpec]
    budget: BudgetSpec
    retry: RetrySpec
    cache: CacheSpec
    observe: ObserveSpec


class ErrorReport(TypedDict):
    error_class: str


class Seen(TypedDict):
    values: dict[str, JsonValue]
    present: dict[str, bool]


class Answer(TypedDict):
    result: JsonValue
    error: ErrorReport | None
    calls: int
    seen: list[Seen]


class Recorder:
    def __init__(self) -> None:
        self.script: list[StepSpec] = []
        self.observe: ObserveSpec = {}
        self.seen: list[Seen] = []
        self.calls = 0

    def reset(self, input_value: Input) -> None:
        outcomes = input_value.get("outcomes")
        observe = input_value.get("observe")
        self.script = [] if outcomes is None else [*outcomes]
        self.observe = {} if observe is None else observe
        self.seen = []
        self.calls = 0

    def enter(self, kwargs: dict[str, JsonValue]) -> None:
        self.calls += 1
        self.seen.append(record(kwargs, self.observe))

    def reply(self) -> JsonValue:
        step = self.script.pop(0)
        if step.get("kind") == "error":
            raise scripted_error(step)
        return step.get("value")


RECORDER: Final = Recorder()


def completion(**kwargs: JsonValue) -> JsonValue:
    RECORDER.enter(kwargs)
    return RECORDER.reply()


async def acompletion(**kwargs: JsonValue) -> JsonValue:
    RECORDER.enter(kwargs)
    return RECORDER.reply()


def record(kwargs: dict[str, JsonValue], observe: ObserveSpec) -> Seen:
    values: dict[str, JsonValue] = {}
    present: dict[str, bool] = {}
    picked = observe.get("values")
    if picked is not None:
        for key in picked:
            values[key] = kwargs.get(key)
    checked = observe.get("present")
    if checked is not None:
        for key in checked:
            present[key] = key in kwargs
    return {"values": values, "present": present}


def scripted_error(step: StepSpec) -> Exception:
    error_class = step.get("error_class")
    message = step.get("message")
    text = "" if message is None else message
    if error_class == "BadRequestError":
        return litellm.BadRequestError(message=text, model="", llm_provider="openai")
    if error_class == "ContextWindowExceededError":
        return litellm.ContextWindowExceededError(message=text, model="", llm_provider="openai")
    if error_class == "APIError":
        return litellm.APIError(status_code=500, message=text, llm_provider="openai", model="")
    return Exception(text)


class Seeded(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.entries: dict[str, JsonValue] = {}

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.entries.get(key)

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.entries.get(key)


def result_value(value: JsonValue) -> JsonValue:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1797-L1800
        JsonValue, to_jsonable_python(value)
    )


def seeded_cache(spec: CacheSpec, request: dict[str, JsonValue]) -> Cache:
    cache = Cache(type=LiteLLMCacheType.LOCAL)
    seeded = Seeded()
    seeded.entries[request_cache_key(cache, request)] = spec.get("hit")
    cache.cache = seeded
    return cache


def request_cache_key(cache: Cache, request: dict[str, JsonValue]) -> str:
    return cache.get_cache_key(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L358
        **request,
    )


class StubFunction(Protocol):
    __name__: str

    def __call__(self, **kwargs: JsonValue) -> JsonValue: ...


class AsyncStubFunction(Protocol):
    __name__: str

    def __call__(self, **kwargs: JsonValue) -> Awaitable[JsonValue]: ...


class SyncWrapper(Protocol):
    def __call__(self, **kwargs: JsonValue) -> JsonValue: ...


class AsyncWrapper(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Awaitable[JsonValue]: ...


class ClientDecorator(Protocol):
    def __call__(self, original_function: StubFunction) -> SyncWrapper: ...


class AsyncClientDecorator(Protocol):
    def __call__(self, original_function: AsyncStubFunction) -> AsyncWrapper: ...


class ModuleState(Protocol):
    max_budget: float
    num_retries: int | None
    num_retries_per_request: int | None
    cache: Cache | None
    _current_cost: float


class RetryEntries(Protocol):
    completion_with_retries: StubFunction
    acompletion_with_retries: AsyncStubFunction


SYNC_CLIENT: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1687
    ClientDecorator, litellm_utils.client  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1687
)
ASYNC_CLIENT: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1675
    AsyncClientDecorator, litellm_utils.client  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1675
)
STATE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L402-L418
    ModuleState, litellm
)
RETRIES: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1911-L1932
    RetryEntries, litellm
)

RETRIES.completion_with_retries = completion
RETRIES.acompletion_with_retries = acompletion


def install_state(input_value: Input) -> None:
    budget = input_value.get("budget")
    retry = input_value.get("retry")
    cache = input_value.get("cache")
    max_budget = None if budget is None else budget.get("max_budget")
    current_cost = None if budget is None else budget.get("current_cost")
    STATE.max_budget = 0.0 if max_budget is None else max_budget
    STATE._current_cost = 0.0 if current_cost is None else current_cost  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L418
    STATE.num_retries = None if retry is None else retry.get("num_retries")
    STATE.num_retries_per_request = (
        None if retry is None else retry.get("num_retries_per_request")
    )
    STATE.cache = None if cache is None else seeded_cache(cache, input_value["request"])
