import os
import sys
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from types import ModuleType
from typing import Protocol, Self

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai import auth as real_auth
from pydantic import BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import TypedDict, override

litellm.suppress_debug_info = True


class Probe(TypedDict, total=False):
    body: str
    status: int
    headers: dict[str, str]
    now: int
    event: dict[str, JsonValue]
    chunks: list[str]
    model: str | None
    stream: bool | None
    provider: str | None
    native: str
    api_key: str | None
    litellm_api_key: str | None
    litellm_openai_key: str | None
    env_api_key: str | None
    api_base: str | None
    litellm_api_base: str | None
    env_base_url: str | None
    env_api_base: str | None
    identity: bool


class Probes(TypedDict):
    probes: list[Probe]


class PostCall(TypedDict):
    original_response: str
    additional_args: dict[str, JsonValue]


class HiddenParams(TypedDict):
    additional_headers: dict[str, JsonValue]


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    params: HiddenParams = Field(alias="_hidden_params")


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None


class Recorded(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: PostCall


class ResponsesConfig(Protocol):
    def transform_streaming_response(
        self, model: str, parsed_chunk: dict[str, JsonValue], logging_obj: Logging
    ) -> BaseModel: ...

    def validate_environment(
        self, headers: dict[str, JsonValue], model: str, litellm_params: GenericLiteLLMParams | None
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def should_fake_stream(
        self, model: str | None, *, stream: bool | None, custom_llm_provider: str | None = None
    ) -> bool: ...


class NativeAnswer:
    def __init__(self, answer: str | None) -> None:
        self.answer = answer

    def __call__(self, model: str, custom_llm_provider: str | None) -> bool:
        if self.answer not in ("true", "false"):
            raise ValueError(f"catalog lookup failed: {model} {custom_llm_provider}")
        return self.answer == "true"


class IdentityAuth(BaseModel):
    workload_identity: dict[str, str]

    def get_token(self) -> str:
        return self.workload_identity["provider"]


class TokenProvider(str):
    __slots__ = ()

    def __new__(cls, token_file: str) -> Self:
        return super().__new__(cls, f"wi:{token_file}")


class FakeSdk(ModuleType):
    @override
    def __getattr__(self, name: str) -> type[IdentityAuth] | type[TokenProvider]:
        if name == "WorkloadIdentityAuth":
            return IdentityAuth
        if name == "k8s_service_account_token_provider":
            return TokenProvider
        raise AttributeError(name)


@contextmanager
def environment(values: dict[str, str]) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    sys.modules[real_auth.__name__] = FakeSdk(real_auth.__name__)
    try:
        yield
    finally:
        sys.modules[real_auth.__name__] = real_auth
        os.environ.clear()
        os.environ.update(saved)


def lookup() -> ResponsesConfig:
    config = ProviderConfigManager.get_provider_responses_api_config(provider=LlmProviders.OPENAI)
    if config is None:
        raise ValueError("the OpenAI responses config is not registered")
    return config


def logging_object() -> Logging:
    return Logging(
        model="gpt-4o",
        messages=None,
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def reply_value(probe: Probe) -> JsonValue:
    body = probe.get("body")
    status = probe.get("status")
    headers = probe.get("headers")
    if body is None or status is None or headers is None:
        raise ValueError("a reply probe needs body, status and headers")
    logging_obj = logging_object()
    request = httpx.Request("POST", "https://api.openai.com/v1/responses")
    response = httpx.Response(
        status, stream=httpx.ByteStream(body.encode()), headers=headers, request=request
    )
    response.read()
    base = ProviderConfigManager.get_provider_responses_api_config(provider=LlmProviders.OPENAI)
    if base is None:
        raise ValueError("no OpenAI responses config")
    started = int(time.time())
    try:
        reply = base.transform_response_api_response(
            model="gpt-4o", raw_response=response, logging_obj=logging_obj
        )
    except Exception as error:
        status = Failure.model_validate(error).status_code
        if status is None:
            raise
        return {"error": status}
    finished = int(time.time())
    hidden = Hidden.model_validate(reply)
    post_call = Recorded.model_validate(logging_obj).model_call_details
    response_headers: dict[str, JsonValue] = dict(response.headers)
    return {
        "id": reply.id,
        "created_at": "now" if started <= reply.created_at <= finished else reply.created_at,
        "headers": response_headers,
        "additional_headers": hidden.params["additional_headers"],
        "post_call": {
            "original_response": post_call["original_response"],
            "additional_args": post_call["additional_args"],
        },
    }


def event_value(probe: Probe) -> JsonValue:
    chunk = probe.get("event")
    if chunk is None:
        raise ValueError("an event probe needs event")
    event = lookup().transform_streaming_response(
        model="gpt-4o", parsed_chunk=chunk, logging_obj=logging_object()
    )
    dumped: dict[str, JsonValue] = event.model_dump(exclude_none=True)
    return {"class": type(event).__name__, "event": dumped}


def terminal_chunks(probe: Probe) -> list[str]:
    chunks = probe.get("chunks")
    if chunks is None:
        raise ValueError("a terminal probe needs chunks")
    return chunks


def terminal_response_value(probe: Probe) -> JsonValue:
    response = OpenAIResponsesAPIConfig.parse_terminal_response_from_stream_chunks(
        terminal_chunks(probe)
    )
    if response is None:
        return None
    return {"id": response.id, "created_at": response.created_at}


def terminal_event_value(probe: Probe) -> JsonValue:
    event = OpenAIResponsesAPIConfig.parse_terminal_event_from_stream_chunks(terminal_chunks(probe))
    if event is None:
        return None
    response = event.response
    return {"class": type(event).__name__, "id": response.id, "created_at": response.created_at}


def stream_value(probe: Probe) -> JsonValue:
    original = litellm.utils.supports_native_streaming
    litellm.utils.supports_native_streaming = NativeAnswer(probe.get("native"))
    try:
        return lookup().should_fake_stream(
            model=probe.get("model"),
            stream=probe.get("stream"),
            custom_llm_provider=probe.get("provider"),
        )
    finally:
        litellm.utils.supports_native_streaming = original


def auth_value(probe: Probe) -> JsonValue:
    given_headers = probe.get("headers")
    headers: dict[str, JsonValue] = {} if given_headers is None else dict(given_headers)
    litellm.api_key = probe.get("litellm_api_key")
    litellm.openai_key = probe.get("litellm_openai_key")
    litellm.api_base = None
    given_env = probe.get("env_api_key")
    values: dict[str, str] = {} if given_env is None else {"OPENAI_API_KEY": given_env}
    api_base = probe.get("api_base")
    if probe.get("identity") is True:
        values["OPENAI_IDENTITY_PROVIDER_ID"] = "idp-fixture"
        values["OPENAI_SERVICE_ACCOUNT_ID"] = "sa-fixture"
        values["OPENAI_IDENTITY_TOKEN_FILE"] = "default" if api_base is None else api_base
    params = GenericLiteLLMParams(api_key=probe.get("api_key"), api_base=api_base)
    with environment(values):
        return lookup().validate_environment(headers, model="gpt-4o", litellm_params=params)


def url_value(probe: Probe) -> JsonValue:
    litellm.api_base = probe.get("litellm_api_base")
    names = {
        "OPENAI_BASE_URL": probe.get("env_base_url"),
        "OPENAI_API_BASE": probe.get("env_api_base"),
    }
    values = {name: text for name, text in names.items() if text is not None}
    with environment(values):
        return lookup().get_complete_url(api_base=probe.get("api_base"), litellm_params={})


HANDLERS: dict[str, Callable[[Probe], JsonValue]] = {
    "created_at_conversion": reply_value,
    "lax_response_validation": reply_value,
    "missing_error_code": event_value,
    "unknown_generic_event": event_value,
    "known_event_variants": event_value,
    "completed_incomplete_failed": terminal_response_value,
    "last_terminal_wins": terminal_event_value,
    "no_terminal_event": terminal_event_value,
    "non_native_streaming_model": stream_value,
    "responses_auth_headers": auth_value,
    "responses_base_url": url_value,
}


def run(input_value: dict[str, Probes]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("expected one openai_response_events case")
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_response_events case: {name}")
    return [handler(probe) for probe in input_value[name]["probes"]]
