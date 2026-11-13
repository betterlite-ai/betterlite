import os
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import Protocol

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai import workload_identity as src
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.responses import transformation as dst
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.llms.openai.workload_identity import OpenAIWorkloadIdentityConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import TypedDict

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


class PrivateState(TypedDict):
    _hidden_params: HiddenParams


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
        match self.answer:
            case "true":
                return True
            case "false":
                return False
            case _:
                raise ValueError(
                    f"the fixture catalog lookup failed for {model} {custom_llm_provider}"
                )


class TokenFixture:
    def __call__(self, config: OpenAIWorkloadIdentityConfig) -> str:
        return f"wi:{config.token_file}"


def resolve_fixture(
    api_key: str | None, api_base: str | None
) -> OpenAIWorkloadIdentityConfig | None:
    if api_key is not None:
        return None
    token_file = "default" if api_base is None else api_base
    return OpenAIWorkloadIdentityConfig("idp-fixture", "sa-fixture", token_file)


@contextmanager
def environment(values: dict[str, str]) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


@contextmanager
def workload_identity(*, enabled: bool) -> Generator[None]:
    if not enabled:
        yield
        return
    dst.resolve_openai_workload_identity_config = resolve_fixture  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L594
    dst.get_workload_identity_bearer_token = TokenFixture()  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L599
    try:
        yield
    finally:
        dst.resolve_openai_workload_identity_config = src.resolve_openai_workload_identity_config  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L594
        dst.get_workload_identity_bearer_token = src.get_workload_identity_bearer_token  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L599


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
        raise ValueError("the OpenAI responses config is not registered")
    started = int(time.time())
    try:
        reply = base.transform_response_api_response(
            model="gpt-4o", raw_response=response, logging_obj=logging_obj
        )
    except OpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L573
        return {"error": error.status_code}
    finished = int(time.time())
    private = TypeAdapter(PrivateState).validate_python(reply.__pydantic_private__)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L585
    post_call = TypeAdapter(PostCall).validate_python(logging_obj.model_call_details)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L566
    response_headers: dict[str, JsonValue] = dict(response.headers)
    return {
        "id": reply.id,
        "created_at": "now" if started <= reply.created_at <= finished else reply.created_at,
        "headers": response_headers,
        "additional_headers": private["_hidden_params"]["additional_headers"],
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
    return {
        "class": type(event).__name__,
        "id": event.response.id,
        "created_at": event.response.created_at,
    }


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
    params = GenericLiteLLMParams(api_key=probe.get("api_key"), api_base=probe.get("api_base"))
    with environment(values), workload_identity(enabled=probe.get("identity") is True):
        return lookup().validate_environment(headers, model="gpt-4o", litellm_params=params)


def url_value(probe: Probe) -> JsonValue:
    litellm.api_base = probe.get("litellm_api_base")
    values: dict[str, str] = {}
    env_base_url = probe.get("env_base_url")
    if env_base_url is not None:
        values["OPENAI_BASE_URL"] = env_base_url
    env_api_base = probe.get("env_api_base")
    if env_api_base is not None:
        values["OPENAI_API_BASE"] = env_api_base
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
