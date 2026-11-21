import asyncio
import logging
from collections.abc import Sequence
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import litellm
from litellm.litellm_core_utils.custom_logger_registry import CustomLoggerRegistry
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitResponse,
    RateLimitStatus,
    RequestRateLimiterStash,
    get_or_create_request_stash,
)
from litellm.types.caching import RedisPipelineIncrementOperation as PipelineIncrement
from litellm.types.utils import ModelResponse, PriorityReservationDict, Usage
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

CALL_ID: Final = "b3053-call"
PRIORITY_LOG_PREFIX: Final = "[Dynamic Rate Limiter] Incremented tokens by"


class HiddenParams(TypedDict):
    additional_headers: dict[str, JsonValue]


class PostCallJson(TypedDict):
    function: Literal["post_call"]
    team_metadata: dict[str, JsonValue] | None
    key_metadata: dict[str, JsonValue]
    statuses: list[RateLimitStatus]
    hidden_params: bool
    headers: dict[str, JsonValue]
    probe_raises: bool


class UsageJson(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class LogSuccessJson(TypedDict):
    function: Literal["log_success"]
    auth_metadata: dict[str, JsonValue] | None
    model_group: str | None
    usage: UsageJson | None
    rate_limit_type: Literal["output", "input", "total"]
    window_size: int
    priority_reservation: dict[str, float | PriorityReservationDict] | None


Input: TypeAlias = PostCallJson | LogSuccessJson


class HeadersAnswer(TypedDict):
    headers: dict[str, JsonValue] | None


class PipelineJson(TypedDict):
    key: str
    increment_value: float
    ttl: int | None


class PipelinesAnswer(TypedDict):
    pipelines: list[list[PipelineJson]]
    priority_logged: str | None


class CallbackArguments(TypedDict, total=False):
    litellm_params: dict[str, JsonValue]
    standard_logging_object: dict[str, JsonValue]


class ProbeUnavailableError(RuntimeError):
    pass


class ProbeResponse:
    def __init__(self, *, hidden: bool, headers: dict[str, JsonValue], raises: bool) -> None:
        self.headers = headers
        self.raises = raises
        if hidden:
            self._hidden_params: HiddenParams = {"additional_headers": headers}

    def __getattr__(self, name: str) -> HiddenParams:
        if name != "_hidden_params":
            raise AttributeError(name)
        if self.raises:
            raise ProbeUnavailableError("the response probe is unavailable")
        raise AttributeError(name)


class V3Limiter(Protocol):
    window_size: int


class DynamicHandler(Protocol):
    v3_limiter: V3Limiter

    async def async_post_call_success_hook(
        self,
        data: dict[str, JsonValue],
        user_api_key_dict: UserAPIKeyAuth,
        response: ProbeResponse,
    ) -> None: ...

    async def async_log_success_event(
        self,
        kwargs: CallbackArguments,
        response_obj: ModelResponse,
        start_time: float,
        end_time: float,
    ) -> None: ...


class RecordingCache:
    redis_cache: None = None

    def __init__(self) -> None:
        self.pipelines: list[list[PipelineJson]] = []

    async def async_increment_cache_pipeline(
        self,
        increment_list: Sequence[PipelineIncrement],
        litellm_parent_otel_span: JsonValue = None,
    ) -> None:
        if litellm_parent_otel_span is not None:
            raise ProbeUnavailableError("the token increments carry a parent span")
        batch = [pipeline_json(entry) for entry in increment_list]
        self.pipelines.append(batch)


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


@runtime_checkable
class DynamicConstructor(Protocol):
    def __call__(self, internal_usage_cache: RecordingCache) -> DynamicHandler: ...


def make_handler(cache: RecordingCache) -> DynamicHandler:
    adapter = TypeAdapter[DynamicConstructor](
        DynamicConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    )
    constructor = adapter.validate_python(
        CustomLoggerRegistry.get_class_type_for_custom_logger_name("dynamic_rate_limiter_v3")
    )
    return constructor(cache)


def pipeline_json(operation: PipelineIncrement) -> PipelineJson:
    return PipelineJson(
        key=operation["key"],
        increment_value=float(operation["increment_value"]),
        ttl=operation["ttl"],
    )


def probe_response(call: PostCallJson) -> ProbeResponse:
    if call["probe_raises"]:
        return ProbeResponse(hidden=False, headers={}, raises=True)
    if call["hidden_params"]:
        return ProbeResponse(hidden=True, headers=dict(call["headers"]), raises=False)
    return ProbeResponse(hidden=False, headers={}, raises=False)


def reported_headers(call: PostCallJson, response: ProbeResponse) -> dict[str, JsonValue] | None:
    if call["probe_raises"]:
        return None
    if not call["hidden_params"]:
        return None
    return response.headers


async def post_call_run(call: PostCallJson, handler: DynamicHandler) -> HeadersAnswer:
    stash: RequestRateLimiterStash = get_or_create_request_stash()
    stash.rate_limit_response = RateLimitResponse(overall_code="OK", statuses=call["statuses"])
    response = probe_response(call)
    auth = UserAPIKeyAuth(metadata=call["key_metadata"], team_metadata=call["team_metadata"])
    await handler.async_post_call_success_hook({"litellm_call_id": CALL_ID}, auth, response)
    return HeadersAnswer(headers=reported_headers(call, response))


def post_call_answer(call: PostCallJson) -> HeadersAnswer:
    handler = make_handler(RecordingCache())
    with asyncio.Runner() as runner:
        return runner.run(post_call_run(call, handler))


def success_kwargs(call: LogSuccessJson) -> CallbackArguments:
    litellm_params: dict[str, JsonValue] = {}
    group = call["model_group"]
    if group is not None:
        litellm_params["metadata"] = {"model_group": group}
    logging_metadata: dict[str, JsonValue] = {}
    auth = call["auth_metadata"]
    if auth is not None:
        logging_metadata["user_api_key_auth_metadata"] = auth
    return CallbackArguments(
        litellm_params=litellm_params,
        standard_logging_object={"metadata": logging_metadata},
    )


async def log_success_run(call: LogSuccessJson, handler: DynamicHandler) -> None:
    handler.v3_limiter.window_size = call["window_size"]
    proxy_server.general_settings = {"token_rate_limit_type": call["rate_limit_type"]}
    litellm.priority_reservation = call["priority_reservation"]
    usage_input = call["usage"]
    usage_value: Usage | None = None
    if usage_input is not None:
        usage_value = Usage(
            prompt_tokens=usage_input["prompt_tokens"],
            completion_tokens=usage_input["completion_tokens"],
            total_tokens=usage_input["total_tokens"],
        )
    response = ModelResponse(usage=usage_value)
    await handler.async_log_success_event(success_kwargs(call), response, 0.0, 0.0)


def logged_priority(messages: list[str]) -> str | None:
    found: str | None = None
    for message in messages:
        if message.startswith(PRIORITY_LOG_PREFIX):
            found = message.rsplit("priority=", 1)[-1]
    return found


def log_success_answer(call: LogSuccessJson) -> PipelinesAnswer:
    cache = RecordingCache()
    handler = make_handler(cache)
    logger = logging.getLogger("LiteLLM Proxy")
    capture = LogCapture()
    saved_handlers = logger.handlers
    saved_level = logger.level
    saved_propagate = logger.propagate
    logger.handlers = []
    logger.addHandler(capture)
    logger.setLevel(logging.DEBUG)
    logger.propagate = False
    try:
        with asyncio.Runner() as runner:
            runner.run(log_success_run(call, handler))
    finally:
        logger.handlers = saved_handlers
        logger.setLevel(saved_level)
        logger.propagate = saved_propagate
    return PipelinesAnswer(
        pipelines=cache.pipelines,
        priority_logged=logged_priority(capture.messages),
    )


def run(input_value: Input) -> HeadersAnswer | PipelinesAnswer:
    if input_value["function"] == "post_call":
        return post_call_answer(input_value)
    return log_success_answer(input_value)


litellm.suppress_debug_info = True
