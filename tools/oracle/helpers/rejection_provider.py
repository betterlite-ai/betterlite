import logging
from collections.abc import Iterator
from typing import Final, Literal, TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy.hooks.rate_limiter_utils import resolve_llm_provider_for_rate_limit
from litellm.router import Router
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue
from typing_extensions import override

# the rate-limit hooks log through this logger, not the "LiteLLM" one
LOGGER_NAME: Final = "LiteLLM Proxy"


class SuccessStep(TypedDict):
    model: str
    outcome: Literal["success"]
    resolved: str
    provider: str


class FailureStep(TypedDict):
    model: str
    outcome: Literal["failure"]


ResolutionStep = SuccessStep | FailureStep


class Input(TypedDict):
    model: str | None
    router: Literal["absent", "empty", "unreadable", "deployments"]
    deployments: list[JsonValue]
    resolutions: list[ResolutionStep]


class Output(TypedDict):
    model: str
    provider: str
    calls: list[str]
    debugs: int


class ProviderLookupError(Exception):
    def __init__(self, model: str) -> None:
        super().__init__(f"no provider resolution is scripted for {model}")


class ExplodingList(list[JsonValue]):
    @override
    def __iter__(self) -> Iterator[JsonValue]:
        raise ProviderLookupError("the router's model list")


class FixtureRouter(Router):
    def __init__(self, model_list: list[JsonValue]) -> None:
        self.model_list = model_list


class Lookup:
    def __init__(self, resolutions: list[ResolutionStep]) -> None:
        self.pending: dict[str, list[ResolutionStep]] = {}
        for step in resolutions:
            self.pending.setdefault(step["model"], []).append(step)
        self.calls: list[str] = []

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
        litellm_params: GenericLiteLLMParams | None = None,
    ) -> tuple[str, str, str | None, str | None]:
        del custom_llm_provider, api_base, api_key, litellm_params
        self.calls.append(model)
        queue = self.pending.get(model)
        if not queue:
            raise ProviderLookupError(model)
        step = queue.pop(0)
        match step["outcome"]:
            case "success":
                return (step["resolved"], step["provider"], None, None)
            case "failure":
                raise ProviderLookupError(model)


class DebugCounter(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.count = 0

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno == logging.DEBUG:
            self.count += 1


def run(input_value: Input) -> Output:
    lookup = Lookup(input_value["resolutions"])
    litellm.get_llm_provider = lookup
    deployments = input_value["deployments"]
    match input_value["router"]:
        case "absent":
            proxy_server.llm_router = None
        case "empty":
            proxy_server.llm_router = FixtureRouter([])
        case "unreadable":
            proxy_server.llm_router = FixtureRouter(ExplodingList(deployments))
        case "deployments":
            proxy_server.llm_router = FixtureRouter(deployments)
    logger = logging.getLogger(LOGGER_NAME)
    counter = DebugCounter()
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [counter]
    logger.setLevel(logging.DEBUG)
    try:
        model, provider = resolve_llm_provider_for_rate_limit(model=input_value["model"])
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
    return {"model": model, "provider": provider, "calls": lookup.calls, "debugs": counter.count}
