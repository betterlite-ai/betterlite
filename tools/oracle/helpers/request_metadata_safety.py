from collections.abc import Callable
from typing import Final, Literal, TypedDict, cast

from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.litellm_pre_call_utils import (
    _get_anthropic_session_id_from_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L776
    _key_or_team_allows_client_message_redaction_opt_out,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L527
    _key_or_team_allows_client_mock_response,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L518
    _key_or_team_allows_client_pricing_override,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L536
    _key_or_team_metadata_flag_is_true,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L508
    _normalized_metadata_object,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L453
    _normalized_metadata_slot,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L468
    _promoted_trace_control_fields,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L665
    _sanitize_for_log,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L171
    _stampable_key_hash,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L154
    _strip_client_callback_credentials,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L563
    _strip_client_message_redaction_opt_out,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L545
    _strip_client_pricing_overrides,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L596
    _strip_router_reserved_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L627
    _strip_untrusted_request_header_controls,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L485
    apply_missing_session_id_policy,
    get_chain_id_from_headers,
    metadata_variable_name_for_route,
)
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

JSON: Final = TypeAdapter(JsonValue)


class SanitizePrincipal(TypedDict):
    metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None


class SanitizeSettings(TypedDict):
    missing_session_id: Literal["omit"] | None
    turn_off_message_logging: bool


class SanitizeCall(TypedDict):
    function: Literal["sanitize"]
    data: dict[str, JsonValue]
    headers: list[list[str]]
    route: str
    principal: SanitizePrincipal
    settings: SanitizeSettings


class HeadersCall(TypedDict):
    function: Literal["chain_id"]
    headers: list[list[str]]


class MetadataCall(TypedDict):
    function: Literal["anthropic_session_id"]
    metadata: JsonValue


class HashCall(TypedDict):
    function: Literal["stampable_key_hash"]
    api_key: str | None
    via_virtual_key: bool


class ValueCall(TypedDict):
    function: Literal["sanitize_for_log"]
    value: JsonValue


class PromotedCall(TypedDict):
    function: Literal["promoted_trace_control_fields"]
    requester_metadata: dict[str, JsonValue]
    litellm_metadata: dict[str, JsonValue]


class RouteCall(TypedDict):
    function: Literal["metadata_variable_name"]
    route: str


class FlagCall(TypedDict):
    function: Literal["key_or_team_flag"]
    metadata_key: str
    metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None


class MockFlagCall(TypedDict):
    function: Literal["mock_response_flag"]
    metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None


class SlotCall(TypedDict):
    function: Literal["normalized_metadata_slot"]
    data: dict[str, JsonValue]
    metadata_variable_name: Literal["metadata", "litellm_metadata"]


class Envelope(TypedDict):
    function: str


class Calls(TypedDict):
    calls: list[JsonValue]


SANITIZE: Final = TypeAdapter(SanitizeCall)
CHAIN: Final = TypeAdapter(HeadersCall)
SESSION: Final = TypeAdapter(MetadataCall)
HASH: Final = TypeAdapter(HashCall)
LOG_VALUE: Final = TypeAdapter(ValueCall)
PROMOTED: Final = TypeAdapter(PromotedCall)
ROUTE: Final = TypeAdapter(RouteCall)
FLAG: Final = TypeAdapter(FlagCall)
MOCK_FLAG: Final = TypeAdapter(MockFlagCall)
SLOT: Final = TypeAdapter(SlotCall)
ENVELOPE: Final = TypeAdapter(Envelope)
CALLS: Final = TypeAdapter(Calls)


def headers_of(pairs: list[list[str]]) -> dict[str, str]:
    headers: dict[str, str] = {}
    for pair in pairs:
        headers[pair[0]] = pair[1]
    return headers


def request_for(route: str) -> Request:
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": route,
        "query_string": b"",
        "headers": [],
    }
    return Request(scope=scope)


def user_api_key_auth(principal: SanitizePrincipal) -> UserAPIKeyAuth:
    auth = UserAPIKeyAuth()
    if principal["metadata"] is not None:
        auth.metadata = principal["metadata"]
    if principal["team_metadata"] is not None:
        auth.team_metadata = principal["team_metadata"]
    return auth


def proxy_error_facts(error: ProxyException) -> JsonValue:
    return JSON.validate_python(
        {
            "error": {
                "status": int(error.code),
                "type": error.type,
                "param": error.param,
                "code": error.code,
            }
        }
    )


def sanitize_outcome(call: SanitizeCall) -> JsonValue:
    data = dict(cast("dict[str, object]", call["data"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L545
    raw = {
        field: data.pop(field)
        for field in ("metadata", "litellm_metadata")
        if data.get(field) is not None
    }
    for field, value in raw.items():
        data[field] = _normalized_metadata_object(field, value)
    variable = metadata_variable_name_for_route(call["route"])
    if data.get(variable) is None:
        data[variable] = {}
    headers = headers_of(call["headers"])
    principal = user_api_key_auth(call["principal"])
    allow_opt_out = _key_or_team_allows_client_message_redaction_opt_out(principal)
    _strip_untrusted_request_header_controls(
        headers, allow_client_message_redaction_opt_out=allow_opt_out
    )
    apply_missing_session_id_policy(
        data,
        variable,
        {"missing_session_id": call["settings"]["missing_session_id"]},
        request_for(call["route"]),
    )
    strip_bucket_headers(data, allow_opt_out=allow_opt_out)
    if not _key_or_team_allows_client_pricing_override(principal):
        _strip_client_pricing_overrides(data)
    _strip_router_reserved_metadata(data)
    _strip_client_callback_credentials(data)
    if not allow_opt_out and call["settings"]["turn_off_message_logging"]:
        _strip_client_message_redaction_opt_out(data)
    return JSON.validate_python(
        {
            "data": data,
            "headers": [[name, value] for name, value in headers.items()],
            "metadata_variable_name": variable,
        }
    )


def strip_bucket_headers(data: dict[str, object], *, allow_opt_out: bool) -> None:
    for slot in ("metadata", "litellm_metadata"):
        if data.get(slot) is not None:
            bucket = _normalized_metadata_slot(data, slot)
            _strip_untrusted_request_header_controls(
                bucket.get("headers"),
                allow_client_message_redaction_opt_out=allow_opt_out,
            )


def sanitize_call(input_value: JsonValue) -> JsonValue:
    call = SANITIZE.validate_python(input_value)
    try:
        return sanitize_outcome(call)
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return proxy_error_facts(error)


def chain_id_call(input_value: JsonValue) -> JsonValue:
    call = CHAIN.validate_python(input_value)
    return get_chain_id_from_headers(headers_of(call["headers"]))


def session_id_call(input_value: JsonValue) -> JsonValue:
    call = SESSION.validate_python(input_value)
    return _get_anthropic_session_id_from_metadata(call["metadata"])


def hash_call(input_value: JsonValue) -> JsonValue:
    call = HASH.validate_python(input_value)
    auth = UserAPIKeyAuth()
    auth.api_key = call["api_key"]
    auth.via_virtual_key = call["via_virtual_key"]
    return _stampable_key_hash(auth)


def log_value_call(input_value: JsonValue) -> JsonValue:
    call = LOG_VALUE.validate_python(input_value)
    return _sanitize_for_log(call["value"])


def promoted_call(input_value: JsonValue) -> JsonValue:
    call = PROMOTED.validate_python(input_value)
    promoted = _promoted_trace_control_fields(
        call["requester_metadata"], call["litellm_metadata"]
    )
    return JSON.validate_python([[key, value] for key, value in promoted])


def route_name_call(input_value: JsonValue) -> JsonValue:
    call = ROUTE.validate_python(input_value)
    return metadata_variable_name_for_route(call["route"])


def flag_call(input_value: JsonValue) -> JsonValue:
    call = FLAG.validate_python(input_value)
    auth = UserAPIKeyAuth()
    if call["metadata"] is not None:
        auth.metadata = call["metadata"]
    if call["team_metadata"] is not None:
        auth.team_metadata = call["team_metadata"]
    return _key_or_team_metadata_flag_is_true(auth, call["metadata_key"])


def mock_flag_call(input_value: JsonValue) -> JsonValue:
    call = MOCK_FLAG.validate_python(input_value)
    auth = UserAPIKeyAuth()
    if call["metadata"] is not None:
        auth.metadata = call["metadata"]
    if call["team_metadata"] is not None:
        auth.team_metadata = call["team_metadata"]
    return _key_or_team_allows_client_mock_response(auth)


def slot_call(input_value: JsonValue) -> JsonValue:
    call = SLOT.validate_python(input_value)
    data = dict(cast("dict[str, object]", call["data"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L468
    _normalized_metadata_slot(data, call["metadata_variable_name"])
    return JSON.validate_python(data)


RUNNERS: dict[str, Callable[[JsonValue], JsonValue]] = {
    "sanitize": sanitize_call,
    "chain_id": chain_id_call,
    "anthropic_session_id": session_id_call,
    "stampable_key_hash": hash_call,
    "sanitize_for_log": log_value_call,
    "promoted_trace_control_fields": promoted_call,
    "metadata_variable_name": route_name_call,
    "key_or_team_flag": flag_call,
    "mock_response_flag": mock_flag_call,
    "normalized_metadata_slot": slot_call,
}


def run(input_value: JsonValue) -> JsonValue:
    calls = CALLS.validate_python(input_value)
    results: list[JsonValue] = []
    for call in calls["calls"]:
        function = ENVELOPE.validate_python(call)["function"]
        results.append(RUNNERS[function](call))
    return results
