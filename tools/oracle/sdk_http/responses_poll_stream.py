import asyncio
import contextlib
import json
from collections.abc import Iterator
from datetime import UTC, datetime
from functools import partial

from fastapi import Request, Response
from litellm.caching.redis_cache import RedisCache
from litellm.proxy.auth.user_api_key_auth import UserAPIKeyAuth
from litellm.proxy.proxy_server import (  # pyright: ignore[reportAny, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L10082-L10096
    ProxyConfig,
    proxy_logging_obj,
    select_data_generator,
)
from litellm.proxy.response_polling import background_streaming
from litellm.proxy.response_polling.background_streaming import background_streaming_task
from litellm.proxy.response_polling.polling_handler import ResponsePollingHandler
from litellm.router import Router
from litellm.utils import Rules, function_setup
from opentelemetry.trace import Span
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.types import Message
from typing_extensions import override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
JSON_OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
CALL_ID = "b936-call"
STATUS_CANCELLED = "cancelled"
LITELLM_VERSION = "1.105.1"


class ResponsesPollStreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str
    polling_id: str
    data: dict[str, JsonValue]
    seed: dict[str, JsonValue]
    clock_millis: list[int]
    cancel_after_write: int | None = None


class Outcome(BaseModel):
    writes: list[JsonValue]


class Value(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    responses_poll_stream: Value


class ResponsesPollStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_poll_stream: ResponsesPollStreamCall

    def serialize(self, api_base: str) -> str:
        writes = run_call(self.responses_poll_stream, api_base)
        envelope = Envelope(responses_poll_stream=Value(value=Outcome(writes=writes)))
        return envelope.model_dump_json(by_alias=True)


# the source's only clock is get_event_loop().time(), so the case scripts its readings
class ScriptedLoop:
    def __init__(self, readings: list[float]) -> None:
        self.readings = readings
        self.read = 0

    def time(self) -> float:
        position = min(self.read, len(self.readings) - 1)
        self.read += 1
        return self.readings[position]


class ScriptedAsyncio:
    Event = asyncio.Event

    def __init__(self, readings: list[float]) -> None:
        self.loop = ScriptedLoop(readings)

    def get_event_loop(self) -> ScriptedLoop:
        return self.loop


# RedisCache.__init__ builds a real client, so skip it and answer every cache call from the script
class CaptureCache(RedisCache):
    def __init__(
        self,
        key: str,
        seed: dict[str, JsonValue],
        cancel_after_write: int | None,
    ) -> None:
        object.__init__(self)
        self.key = key
        self.entries: dict[str, str] = {key: json.dumps(seed)}
        self.writes: list[JsonValue] = []
        self.cancel_after_write = cancel_after_write

    @override
    async def async_set_cache(self, key: str, value: str, **kwargs: int) -> None:
        del kwargs
        self.entries[key] = value
        self.writes.append(JSON.validate_json(value))
        if self.cancel_after_write == len(self.writes):
            # the cancel route writes the same key from outside the worker
            state = JSON_OBJECT.validate_json(self.entries[key])
            state["status"] = STATUS_CANCELLED
            self.entries[key] = json.dumps(state)

    @override
    async def async_get_cache(
        self, key: str, parent_otel_span: Span | None = None, **kwargs: int
    ) -> str | None:
        del parent_otel_span, kwargs
        return self.entries.get(key)


async def disconnected_receive() -> Message:
    return {"type": "http.disconnect"}


def request_scope(call: ResponsesPollStreamCall) -> dict[str, object]:
    return {
        "type": "http",
        "http_version": "1.1",
        "method": "POST",
        "scheme": "http",
        "path": "/v1/responses",
        "raw_path": b"/v1/responses",
        "query_string": b"",
        "headers": [
            (b"authorization", f"Bearer {call.api_key}".encode()),
            (b"content-type", b"application/json"),
        ],
        "client": ("127.0.0.1", 50123),
        "server": ("127.0.0.1", 80),
        "root_path": "",
        "app": None,
    }


def requested_model(call: ResponsesPollStreamCall) -> str:
    model = call.data.get("model")
    if not isinstance(model, str):
        raise ValueError("data.model must be a string")
    return model


def provider_router(call: ResponsesPollStreamCall, api_base: str) -> Router:
    model = requested_model(call)
    return Router(
        model_list=[
            {
                "model_name": model,
                "litellm_params": {
                    "model": f"openai/{model}",
                    "api_base": api_base,
                    "api_key": call.api_key,
                },
            }
        ]
    )


# the request body the proxy already read, plus the logging object its pre-call logic built
def prepared_request(call: ResponsesPollStreamCall) -> dict[str, object]:
    request_data: dict[str, JsonValue] = {**call.data, "litellm_call_id": CALL_ID}
    logging_obj, prepared = function_setup(  # pyright: ignore[reportAny, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L981-L988
        original_function="aresponses",
        rules_obj=Rules(),
        start_time=datetime.now(UTC),
        **request_data,
    )
    prepared["litellm_logging_obj"] = logging_obj
    return dict(prepared)


@contextlib.contextmanager
def scripted_clock(readings: list[float]) -> Iterator[None]:
    original = background_streaming.asyncio
    object.__setattr__(background_streaming, "asyncio", ScriptedAsyncio(readings))
    try:
        yield
    finally:
        object.__setattr__(background_streaming, "asyncio", original)


async def run_worker(
    call: ResponsesPollStreamCall, api_base: str
) -> list[JsonValue]:
    cache = CaptureCache(
        key=ResponsePollingHandler.get_cache_key(call.polling_id),
        seed=call.seed,
        cancel_after_write=call.cancel_after_write,
    )
    handler = ResponsePollingHandler(redis_cache=cache)
    request = Request(request_scope(call), disconnected_receive)
    await background_streaming_task(  # pyright: ignore[reportAny, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/response_polling/background_streaming.py#L96-L162
        polling_id=call.polling_id,
        data=prepared_request(call),
        polling_handler=handler,
        request=request,
        fastapi_response=Response(),
        user_api_key_dict=UserAPIKeyAuth(api_key=call.api_key, user_id="b936"),
        general_settings={},
        llm_router=provider_router(call, api_base),
        proxy_config=ProxyConfig(),
        proxy_logging_obj=proxy_logging_obj,
        select_data_generator=partial(select_data_generator, responses_stream_errors=True),  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L10082-L10096
        user_model=None,
        user_temperature=None,
        user_request_timeout=None,
        user_max_tokens=None,
        user_api_base=None,
        version=LITELLM_VERSION,
    )
    return cache.writes


def run_call(call: ResponsesPollStreamCall, api_base: str) -> list[JsonValue]:
    readings = [millis / 1000.0 for millis in call.clock_millis]
    with scripted_clock(readings):
        return asyncio.run(run_worker(call, api_base))
