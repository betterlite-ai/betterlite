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
from starlette.types import Scope

Slot = Literal["metadata", "litellm_metadata"]
Pairs = list[tuple[str, str]]


class CleanCall(TypedDict):
    headers: Pairs
    litellm_key_header_name: str | None
    forward_llm_provider_auth_headers: bool
    authenticated_with_header: str | None


class GeneralFlags(TypedDict):
    forward_client_headers_to_llm_api: bool
    forward_openai_org_id: bool


class BackendCall(TypedDict):
    headers: Pairs
    request_data: dict[str, JsonValue]
    principal: dict[str, JsonValue]
    general_settings: GeneralFlags
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


class Span(TypedDict):
    trace_id: int
    span_id: int


class OtelCall(TypedDict):
    data: dict[str, JsonValue]
    slot: Slot
    span: Span | None


class TraceparentCall(TypedDict):
    data: dict[str, JsonValue]
    headers: Pairs
    forward: bool
    otel_enabled: bool


class UserHeaderMapping(TypedDict):
    header_name: str
    litellm_user_role: Literal["internal_user", "customer"]


class UserSettings(TypedDict):
    user_header_name: str | None
    user_header_mappings: list[UserHeaderMapping] | None


class UserCall(TypedDict):
    headers: Pairs
    general_settings: UserSettings
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


def request_for(headers: Pairs) -> Request:
    raw_headers = [(name.lower().encode(), value.encode()) for name, value in headers]
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": "/",
        "query_string": b"",
        "headers": raw_headers,
    }
    return Request(scope=scope)


def clean(call: CleanCall) -> JsonValue:
    # the proxy hands clean_headers the request's Headers, which keep each name as the caller sent it
    raw_headers = Headers(raw=[(name.encode(), value.encode()) for name, value in call["headers"]])
    cleaned: dict[str, str] = clean_headers(
        raw_headers,
        litellm_key_header_name=call["litellm_key_header_name"],
        forward_llm_provider_auth_headers=call["forward_llm_provider_auth_headers"],
        authenticated_with_header=call["authenticated_with_header"],
    )
    outcome = {"forwarded": cleaned, "logged": redact_credential_headers(cleaned)}
    return JSON.validate_python(outcome)


def backend(call: BackendCall) -> JsonValue:
    flags = call["general_settings"]
    previous = litellm.add_user_information_to_llm_headers
    litellm.add_user_information_to_llm_headers = call["add_user_information"]
    try:
        arguments: LitellmDataForBackendLLMCall = LiteLLMProxyRequestSetup.add_litellm_data_for_backend_llm_call(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1481
            headers=dict(call["headers"]),
            request_data=call["request_data"],
            user_api_key_dict=AUTH.validate_python(call["principal"]),
            general_settings={
                "forward_client_headers_to_llm_api": flags["forward_client_headers_to_llm_api"],
                "forward_openai_org_id": flags["forward_openai_org_id"],
            },
        )
    except ValueError:
        error = {"error": "invalid header"}
        return JSON.validate_python(error)
    finally:
        litellm.add_user_information_to_llm_headers = previous
    return JSON.validate_python(arguments)


def model_group(call: ModelGroupCall) -> JsonValue:
    principal = AUTH.validate_python(call["principal"])
    principal.team_model_aliases = call["team_model_aliases"]
    models = call["models"]
    previous_settings = litellm.model_group_settings
    previous_flag = litellm.add_user_information_to_llm_headers
    forwarding = None if models is None else ModelGroupSettings(forward_client_headers_to_llm_api=models)
    litellm.model_group_settings = forwarding
    litellm.add_user_information_to_llm_headers = call["add_user_information"]
    data = dict(call["data"])
    try:
        LiteLLMProxyRequestSetup.add_headers_to_llm_call_by_model_group(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1438
            data, dict(call["headers"]), principal
        )
    finally:
        litellm.model_group_settings = previous_settings
        litellm.add_user_information_to_llm_headers = previous_flag
    outcome = {"headers": data["headers"] if "headers" in data else None}
    return JSON.validate_python(outcome)


def metadata(call: MetadataCall) -> JsonValue:
    data = dict(call["data"])
    LiteLLMProxyRequestSetup.add_litellm_metadata_from_request_headers(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1538
        dict(call["headers"]), data, call["slot"]
    )
    return JSON.validate_python(data)


def otel_trace_id(call: OtelCall) -> JsonValue:
    span = call["span"]
    parent = None
    if span is not None:
        parent = NonRecordingSpan(SpanContext(span["trace_id"], span["span_id"], is_remote=False))
    stamped: dict[str, object] = dict(call["data"])
    add_otel_trace_id_to_request(stamped, call["slot"], parent)
    return JSON.validate_python(stamped)


def traceparent(call: TraceparentCall) -> JsonValue:
    previous_logger = proxy_server.open_telemetry_logger
    previous_flag = litellm.forward_traceparent_to_llm_provider
    # only the logger's presence matters here, so it is never started
    proxy_server.open_telemetry_logger = OpenTelemetry.__new__(OpenTelemetry) if call["otel_enabled"] else None
    litellm.forward_traceparent_to_llm_provider = call["forward"]
    data = dict(call["data"])
    try:
        _add_otel_traceparent_to_data(data, request_for(call["headers"]))
    finally:
        proxy_server.open_telemetry_logger = previous_logger
        litellm.forward_traceparent_to_llm_provider = previous_flag
    return JSON.validate_python(data)


def user(call: UserCall) -> JsonValue:
    given = call["general_settings"]
    mappings = given["user_header_mappings"]
    listed: list[JsonValue] | None = None
    if mappings is not None:
        listed = [
            {"header_name": mapping["header_name"], "litellm_user_role": mapping["litellm_user_role"]}
            for mapping in mappings
        ]
    settings: dict[str, JsonValue] = {"user_header_name": given["user_header_name"], "user_header_mappings": listed}
    headers = dict(call["headers"])
    principal = UserAPIKeyAuth(user_id=call["principal_user_id"])
    LiteLLMProxyRequestSetup.add_internal_user_from_user_mapping(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1354
        settings, principal, headers
    )
    found: str | None = LiteLLMProxyRequestSetup.get_user_from_headers(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L1374
        headers, settings
    )
    outcome = {"user": found, "user_id": principal.user_id}
    return JSON.validate_python(outcome)


def provider(headers: Pairs) -> JsonValue:
    data: dict[str, JsonValue] = {}
    used: bool = add_provider_specific_headers_to_request(data, dict(headers))
    outcome = {"data": data, "used_client_oauth_token": used}
    return JSON.validate_python(outcome)


def cache_control(text: str) -> JsonValue:
    try:
        parsed: dict[str, str | bool] = parse_cache_control(text)
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
