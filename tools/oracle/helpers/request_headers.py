from typing import Final, Literal, TypedDict

import litellm
from litellm.integrations.opentelemetry import OpenTelemetry
from litellm.proxy import proxy_server
from litellm.proxy._types import LitellmDataForBackendLLMCall, UserAPIKeyAuth
from litellm.proxy.litellm_pre_call_utils import (
    LiteLLMProxyRequestSetup,
    _add_otel_traceparent_to_data,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L3516
    add_otel_trace_id_to_request,
    add_provider_specific_headers_to_request,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L3489
    clean_headers,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1173
    parse_cache_control,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L225
    redact_credential_headers,
)
from litellm.types.router import ModelGroupSettings
from opentelemetry.trace import NonRecordingSpan, SpanContext
from pydantic import JsonValue, TypeAdapter
from starlette.datastructures import Headers
from starlette.requests import Request

Slot = Literal["metadata", "litellm_metadata"]
Pairs = list[tuple[str, str]]


class CleanCall(TypedDict):
    headers: Pairs
    litellm_key_header_name: str | None
    forward_auth_headers: bool
    authenticated_with_header: str | None


# general_settings travels as the proxy keeps it, a dict of the config's keys
class BackendCall(TypedDict):
    headers: Pairs
    request_data: dict[str, JsonValue]
    principal: dict[str, JsonValue]
    general_settings: dict[str, JsonValue]
    add_user_information: bool


class ModelGroupCall(TypedDict):
    data: dict[str, JsonValue]
    headers: Pairs
    principal: dict[str, JsonValue]
    team_model_aliases: dict[str, str]
    models: list[str] | None
    add_user_information: bool


class MetadataCall(TypedDict):
    headers: Pairs
    data: dict[str, JsonValue]
    slot: Slot


# span is the parent span's trace id and span id
class OtelCall(TypedDict):
    data: dict[str, JsonValue]
    slot: Slot
    span: tuple[int, int] | None


class TraceparentCall(TypedDict):
    data: dict[str, JsonValue]
    headers: Pairs
    forward: bool
    otel_enabled: bool


class UserCall(TypedDict):
    headers: Pairs
    general_settings: dict[str, JsonValue]
    principal_user_id: str | None


class Input(TypedDict):
    clean: list[CleanCall]
    backend: list[BackendCall]
    model_group: list[ModelGroupCall]
    metadata: list[MetadataCall]
    otel_trace_id: list[OtelCall]
    traceparent: list[TraceparentCall]
    user: list[UserCall]
    provider: list[Pairs]
    cache_control: list[str]


JSON: Final = TypeAdapter(JsonValue)
AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)


def clean(call: CleanCall) -> JsonValue:
    # the proxy passes clean_headers the request's Headers, so names keep the caller's case
    raw_headers = Headers(raw=[(name.encode(), value.encode()) for name, value in call["headers"]])
    cleaned: dict[str, str] = clean_headers(  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1178
        raw_headers,
        litellm_key_header_name=call["litellm_key_header_name"],
        forward_llm_provider_auth_headers=call["forward_auth_headers"],
        authenticated_with_header=call["authenticated_with_header"],
    )
    outcome = {"forwarded": cleaned, "logged": redact_credential_headers(cleaned)}
    return JSON.validate_python(outcome)


# dispatch.py runs one call per process, so the LiteLLM globals set below never reach another call
def backend(call: BackendCall) -> JsonValue:
    litellm.add_user_information_to_llm_headers = call["add_user_information"]
    headers = dict(call["headers"])
    principal = AUTH.validate_python(call["principal"])
    try:
        arguments: LitellmDataForBackendLLMCall = (
            LiteLLMProxyRequestSetup.add_litellm_data_for_backend_llm_call(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1481
                headers=headers,
                request_data=call["request_data"],
                user_api_key_dict=principal,
                general_settings=call["general_settings"],
            )
        )
    except ValueError:
        error = {"error": "invalid header"}
        return JSON.validate_python(error)
    return JSON.validate_python(arguments)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4980


def model_group(call: ModelGroupCall) -> JsonValue:
    principal = AUTH.validate_python(call["principal"])
    principal.team_model_aliases = call["team_model_aliases"]
    models = call["models"]
    litellm.model_group_settings = None
    if models is not None:
        litellm.model_group_settings = ModelGroupSettings(forward_client_headers_to_llm_api=models)
    litellm.add_user_information_to_llm_headers = call["add_user_information"]
    data = dict(call["data"])
    headers = dict(call["headers"])
    LiteLLMProxyRequestSetup.add_headers_to_llm_call_by_model_group(data, headers, principal)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1438
    outcome = {"headers": data.get("headers")}
    return JSON.validate_python(outcome)


def metadata(call: MetadataCall) -> JsonValue:
    data = dict(call["data"])
    headers = dict(call["headers"])
    LiteLLMProxyRequestSetup.add_litellm_metadata_from_request_headers(headers, data, call["slot"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1538
    return JSON.validate_python(data)


def otel_trace_id(call: OtelCall) -> JsonValue:
    span = call["span"]
    parent = None
    if span is not None:
        parent = NonRecordingSpan(SpanContext(span[0], span[1], is_remote=False))
    stamped = dict[str, object](call["data"])
    add_otel_trace_id_to_request(stamped, call["slot"], parent)
    return JSON.validate_python(stamped)


def traceparent(call: TraceparentCall) -> JsonValue:
    proxy_server.open_telemetry_logger = None
    # only the logger's presence matters here, so it is never started
    if call["otel_enabled"]:
        proxy_server.open_telemetry_logger = OpenTelemetry.__new__(OpenTelemetry)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L381
    litellm.forward_traceparent_to_llm_provider = call["forward"]
    raw_headers = [(name.lower().encode(), value.encode()) for name, value in call["headers"]]
    scope: dict[str, str | list[tuple[bytes, bytes]]] = {"type": "http", "headers": raw_headers}
    data = dict(call["data"])
    _add_otel_traceparent_to_data(data, Request(scope))
    return JSON.validate_python(data)


def user(call: UserCall) -> JsonValue:
    headers = dict(call["headers"])
    principal = UserAPIKeyAuth(user_id=call["principal_user_id"])
    LiteLLMProxyRequestSetup.add_internal_user_from_user_mapping(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1354
        call["general_settings"], principal, headers
    )
    found: str | None = LiteLLMProxyRequestSetup.get_user_from_headers(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1374
        headers, call["general_settings"]
    )
    outcome = {"user": found, "user_id": principal.user_id}
    return JSON.validate_python(outcome)


def provider(pairs: Pairs) -> JsonValue:
    data: dict[str, JsonValue] = {}
    headers = dict(pairs)
    used: bool = add_provider_specific_headers_to_request(data, headers)
    outcome = {"data": data, "used_client_oauth_token": used}
    return JSON.validate_python(outcome)


def cache_control(text: str) -> JsonValue:
    try:
        parsed: dict[str, str | bool] = parse_cache_control(text)  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L225
    except ValueError:
        error = {"error": "invalid directive"}
        return JSON.validate_python(error)
    return JSON.validate_python(parsed)


def run(input_value: Input) -> JsonValue:
    outcome = {
        "clean": [clean(call) for call in input_value["clean"]],
        "backend": [backend(call) for call in input_value["backend"]],
        "model_group": [model_group(call) for call in input_value["model_group"]],
        "metadata": [metadata(call) for call in input_value["metadata"]],
        "otel_trace_id": [otel_trace_id(call) for call in input_value["otel_trace_id"]],
        "traceparent": [traceparent(call) for call in input_value["traceparent"]],
        "user": [user(call) for call in input_value["user"]],
        "provider": [provider(headers) for headers in input_value["provider"]],
        "cache_control": [cache_control(text) for text in input_value["cache_control"]],
    }
    return JSON.validate_python(outcome)
