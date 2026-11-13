import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.volcengine.common_utils import VolcEngineError
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams as Params
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, ValidationError

Headers: TypeAlias = dict[str, str]
ParamsInput: TypeAlias = dict[str, JsonValue]
Supported: TypeAlias = Callable[[str], list[str]]
Sanitize: TypeAlias = Callable[[str, str, ParamsInput, Params, Headers], ParamsInput]
Url: TypeAlias = Callable[[str | None, ParamsInput], str]
Environment: TypeAlias = Callable[[Headers, str, Params | None], ParamsInput]
Streaming: TypeAlias = Callable[[str, ParamsInput, Logging], BaseModel]
CreateReply: TypeAlias = Callable[[str, httpx.Response, Logging], ResponsesAPIResponse]
ResourceReply: TypeAlias = Callable[[httpx.Response, Logging], ResponsesAPIResponse]


class FakeStream(Protocol):
    def __call__(self, model: str | None, *, stream: bool | None) -> bool: ...


class Step(TypedDict, total=False):
    kind: str
    model: str | None
    params: ParamsInput
    api_base: str | None
    global_base: str | None
    volcengine_base: str | None
    ark_base: str | None
    api_key: str | None
    global_key: str | None
    ark_key: str | None
    volcengine_key: str | None
    extra_headers: Headers
    event: ParamsInput
    operation: str
    status: int
    body: str
    headers: Headers
    stream: bool | None


class Input(TypedDict):
    steps: list[Step]


class Config(Protocol):
    @property
    def get_supported_openai_params(self) -> Supported: ...

    @property
    def transform_responses_api_request(self) -> Sanitize: ...  # ast-grep-ignore

    @property
    def get_complete_url(self) -> Url: ...

    @property
    def validate_environment(self) -> Environment: ...

    @property
    def transform_streaming_response(self) -> Streaming: ...

    @property
    def transform_response_api_response(self) -> CreateReply: ...  # ast-grep-ignore

    @property
    def transform_get_response_api_response(self) -> ResourceReply: ...  # ast-grep-ignore

    @property
    def transform_cancel_response_api_response(self) -> ResourceReply: ...  # ast-grep-ignore

    @property
    def should_fake_stream(self) -> FakeStream: ...

    @property
    def supports_native_websocket(self) -> Callable[[], bool]: ...


def config() -> Config:
    found = ProviderConfigManager.get_provider_responses_api_config(
        provider=LlmProviders.VOLCENGINE
    )
    if found is None:
        raise ValueError("the Volcengine responses config is not registered")
    return found


def logging_object() -> Logging:
    return Logging(
        model="volcengine-model", messages=None, stream=False,
        call_type="responses", start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1", function_id="fn-1",
    )


@contextmanager
def configured(
    values: dict[str, str], api_key: str | None, api_base: str | None
) -> Generator[None]:
    saved_env = dict(os.environ)
    saved_key = litellm.api_key
    saved_base = litellm.api_base
    os.environ.clear()
    os.environ.update(values)
    litellm.api_key = api_key
    litellm.api_base = api_base
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved_env)
        litellm.api_key = saved_key
        litellm.api_base = saved_base


def response_of(step: Step) -> httpx.Response:
    status = step.get("status")
    body = step.get("body")
    if status is None or body is None:
        raise ValueError("this step needs status and body")
    headers = step.get("headers")
    request = httpx.Request("POST", "https://ark.example.test/api/v3/responses")
    response = httpx.Response(
        status, headers={} if headers is None else headers, content=body.encode(), request=request
    )
    response.read()
    return response


def supported_params(step: Step) -> JsonValue:
    model = step.get("model")
    if model is None:
        raise ValueError("a supported_params step needs model")
    names: list[JsonValue] = []
    names.extend(config().get_supported_openai_params("" if model == "any" else model))
    return names


def sanitize(step: Step) -> JsonValue:
    params = step.get("params")
    if params is None:
        raise ValueError("a sanitize step needs params")
    body = config().transform_responses_api_request(
        "volcengine-model", "hi", params, Params(), {}
    )
    body.pop("model", None)
    body.pop("input", None)
    return body


def url(step: Step) -> JsonValue:
    values: dict[str, str] = {}
    volcengine_base = step.get("volcengine_base")
    if volcengine_base is not None:
        values["VOLCENGINE_API_BASE"] = volcengine_base
    ark_base = step.get("ark_base")
    if ark_base is not None:
        values["ARK_API_BASE"] = ark_base
    with configured(values, None, step.get("global_base")):
        return config().get_complete_url(step.get("api_base"), {})


def headers(step: Step) -> JsonValue:
    values: dict[str, str] = {}
    ark_key = step.get("ark_key")
    if ark_key is not None:
        values["ARK_API_KEY"] = ark_key
    volcengine_key = step.get("volcengine_key")
    if volcengine_key is not None:
        values["VOLCENGINE_API_KEY"] = volcengine_key
    extra = step.get("extra_headers")
    params = Params(api_key=step.get("api_key"))
    with configured(values, step.get("global_key"), None):
        try:
            merged = config().validate_environment({} if extra is None else extra, "", params)
        except ValueError:
            return {"ok": False}
        return {"ok": True, "headers": merged}


def stream_event(step: Step) -> JsonValue:
    event = step.get("event")
    if event is None:
        raise ValueError("a stream_event step needs event")
    try:
        decoded = config().transform_streaming_response("", event, logging_object())
    except ValidationError:
        return {"ok": False}
    dumped: ParamsInput = decoded.model_dump(exclude_none=True)
    return {"ok": True, "class": type(decoded).__name__, "event": dumped}


def reply_value(step: Step) -> JsonValue:
    response = response_of(step)
    operation = step.get("operation")
    try:
        if operation == "create":
            reply = config().transform_response_api_response(
                "volcengine-model", response, logging_object()
            )
        elif operation == "get":
            reply = config().transform_get_response_api_response(response, logging_object())
        elif operation == "cancel":
            reply = config().transform_cancel_response_api_response(response, logging_object())
        else:
            raise ValueError("a reply step needs create, get or cancel")
    except (ValidationError, VolcEngineError):
        return {"ok": False}
    return {
        "ok": True,
        "id": reply.id,
        "created_at": reply.created_at,
        "headers": dict(response.headers),
    }


def capabilities(step: Step) -> JsonValue:
    stream = step.get("stream")
    if stream is None:
        raise ValueError("a capabilities step needs stream")
    return {
        "fake_stream": config().should_fake_stream(step.get("model"), stream=stream),
        "websocket": config().supports_native_websocket(),
    }


HANDLERS: dict[str, Callable[[Step], JsonValue]] = {
    "supported_params": supported_params,
    "sanitize": sanitize,
    "url": url,
    "headers": headers,
    "stream_event": stream_event,
    "reply": reply_value,
    "capabilities": capabilities,
}


def execute(step: Step) -> JsonValue:
    kind = step.get("kind")
    if kind is None:
        raise ValueError("every step needs a kind")
    handler = HANDLERS.get(kind)
    if handler is None:
        raise ValueError(f"unknown b8176 step kind {kind}")
    return handler(step)


def run(value: Input) -> JsonValue:
    steps = value["steps"]
    return [execute(step) for step in steps]
