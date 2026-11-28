import sys
from collections.abc import Mapping
from types import ModuleType
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

from litellm.llms.anthropic.pass_through.adapters import handler
from pydantic import JsonValue

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
ROUTER_AVAILABLE: Final = "injected-router"
ROUTER_MARKER: Final[dict[str, JsonValue]] = {"injected": True}

RouterMode: TypeAlias = Literal["handle", "empty", "missing"]


class InspectionCall(TypedDict):
    messages: list[JsonValue]
    kwargs: dict[str, JsonValue]
    router: RouterMode


class PredictionCall(TypedDict):
    context_management_spec: JsonValue
    additional_drop_params: JsonValue


class Input(TypedDict):
    inspections: list[InspectionCall]
    predictions: list[PredictionCall]


class BlockCheck(Protocol):
    def __call__(self, messages: list[JsonValue]) -> bool: ...


class ProxyExtract(Protocol):
    def __call__(
        self, kwargs: Mapping[str, JsonValue]
    ) -> tuple[dict[str, JsonValue] | None, JsonValue]: ...


class RouterFallback(Protocol):
    def __call__(self) -> JsonValue: ...


class SpecEdits(Protocol):
    def __call__(
        self, *, context_management_spec: JsonValue, additional_drop_params: JsonValue
    ) -> bool: ...


BLOCK_CHECK: Final = cast(BlockCheck, handler._messages_have_compaction_block)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L51-L60
ROUTER_FALLBACK: Final = cast(RouterFallback, handler._proxy_router_fallback)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L63-L68
METADATA: Final = cast(ProxyExtract, handler._extract_proxy_litellm_metadata)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L71-L89
WILL_RUN: Final = cast(SpecEdits, handler._polyfill_will_run)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L162-L186
HAS_NON_COMPACT: Final = cast(SpecEdits, handler._spec_has_non_compact_edits)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L189-L212


class ProxyServer(ModuleType):
    llm_router: JsonValue = None


def proxy_module(mode: RouterMode) -> ModuleType:
    if mode == "missing":
        return ModuleType(PROXY_MODULE)
    module = ProxyServer(PROXY_MODULE)
    if mode == "handle":
        module.llm_router = ROUTER_MARKER
    return module


def install_proxy(mode: RouterMode) -> ModuleType | None:
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_module(mode)
    return saved


def restore_proxy(saved: ModuleType | None) -> None:
    if saved is None:
        del sys.modules[PROXY_MODULE]
        return
    sys.modules[PROXY_MODULE] = saved


def router_answer(mode: RouterMode) -> JsonValue:
    saved = install_proxy(mode)
    try:
        handle = ROUTER_FALLBACK()
    finally:
        restore_proxy(saved)
    if handle is None:
        return None
    return ROUTER_AVAILABLE


def inspection_answer(call: InspectionCall) -> JsonValue:
    metadata, auth = METADATA(call["kwargs"])
    answer: dict[str, JsonValue] = {
        "has_compaction_block": BLOCK_CHECK(call["messages"]),
        "litellm_metadata": metadata,
        "user_api_key_auth": auth,
        "router": router_answer(call["router"]),
    }
    return answer


def prediction_answer(call: PredictionCall) -> JsonValue:
    spec = call["context_management_spec"]
    drops = call["additional_drop_params"]
    answer: dict[str, JsonValue] = {
        "compact": WILL_RUN(context_management_spec=spec, additional_drop_params=drops),
        "non_compact": HAS_NON_COMPACT(
            context_management_spec=spec, additional_drop_params=drops
        ),
    }
    return answer


def run(input_value: Input) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "inspections": [inspection_answer(call) for call in input_value["inspections"]],
        "predictions": [prediction_answer(call) for call in input_value["predictions"]],
    }
    return answer
