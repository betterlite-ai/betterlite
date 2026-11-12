from collections.abc import Callable
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from fastapi import HTTPException
from litellm.proxy import litellm_pre_call_utils
from litellm.proxy.common_request_processing import proxy_exception_from_http_exception
from litellm.proxy.common_utils.http_parsing_utils import (
    get_tags_from_request_body,
    populate_request_with_path_params,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L654
    resolve_inference_model,
)
from litellm.proxy.litellm_pre_call_utils import reject_url_valued_destination
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message, Scope

Json: TypeAlias = JsonValue
Kind: TypeAlias = Literal[
    "completion", "image_generation", "image_edit", "moderation", "speech", "body", "path"
]


class Defaults(TypedDict, total=False):
    completion_model: str
    image_generation_model: str
    moderation_model: str


class ResolveCall(TypedDict):
    function: Literal["resolve_inference_model"]
    body_model: str | None
    settings: Defaults
    cli_model: str | None
    endpoint_model: str | None
    kind: Kind


class SingleRejectCall(TypedDict):
    function: Literal["reject_url_valued_destination"]
    field: Literal["model", "file_id"]
    value: str
    allowed_hosts: list[str]


class DataRejectCall(TypedDict):
    function: Literal["reject_url_valued_destinations"]
    data: dict[str, Json]
    allowed_hosts: list[str]


class TagsCall(TypedDict):
    function: Literal["tags_from_request_body"]
    request_body: dict[str, Json]


class PopulateCall(TypedDict):
    function: Literal["populate_request_parameters"]
    request_data: dict[str, Json]
    path_params: NotRequired[dict[str, Json] | None]
    query_params: NotRequired[dict[str, str]]
    route_path: str


class Envelope(TypedDict):
    function: str


class Calls(TypedDict):
    calls: list[Json]


ENVELOPE = TypeAdapter(Envelope)
CALLS = TypeAdapter(Calls)
RESOLVE = TypeAdapter(ResolveCall)
SINGLE_REJECT = TypeAdapter(SingleRejectCall)
DATA_REJECT = TypeAdapter(DataRejectCall)
TAGS = TypeAdapter(TagsCall)
POPULATE = TypeAdapter(PopulateCall)


class EmptyReceiver:
    def __init__(self) -> None:
        self.sent = False

    async def __call__(self) -> Message:
        self.sent = True
        return {"type": "http.disconnect"}


def query_string(query_params: dict[str, str] | None) -> bytes:
    if query_params is None:
        return b""
    joined = "&".join(f"{name}={value}" for name, value in query_params.items())
    return joined.encode()


def build_request(call: PopulateCall) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": call["route_path"],
        "query_string": query_string(call.get("query_params")),
        "headers": headers,
    }
    request = Request(scope, EmptyReceiver())
    path_params = call.get("path_params")
    if path_params is not None:
        request.scope["path_params"] = path_params
    return request


def rejection_facts(error: HTTPException) -> Json:
    mapped = proxy_exception_from_http_exception(error, {})
    detail = cast("dict[str, Json]", error.detail)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L423-L435
    facts: Json = {
        "status": int(mapped.code),
        "type": mapped.type,
        "param": mapped.param,
        "code": mapped.code,
        "detail": {"error": detail["error"], "param": detail["param"]},
    }
    return facts


def resolve_call(input_value: Json) -> Json:
    call = RESOLVE.validate_python(input_value)
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L32
        "Json",
        resolve_inference_model(
            call["body_model"],
            call["settings"],
            call["cli_model"],
            call["endpoint_model"],
            kind=call["kind"],
        ),
    )


def reject_call(input_value: Json) -> Json:
    call = SINGLE_REJECT.validate_python(input_value)
    litellm.provider_url_destination_allowed_hosts = call["allowed_hosts"]
    try:
        reject_url_valued_destination(call["field"], call["value"])
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L423
        return rejection_facts(error)
    return None


def reject_data_call(input_value: Json) -> Json:
    call = DATA_REJECT.validate_python(input_value)
    litellm.provider_url_destination_allowed_hosts = call["allowed_hosts"]
    data = cast("dict[str, object]", call["data"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L394-L408
    try:
        litellm_pre_call_utils._reject_url_valued_destinations(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L394
            data
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L423
        return rejection_facts(error)
    return None


def tags_call(input_value: Json) -> Json:
    call = TAGS.validate_python(input_value)
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L618
        "Json",
        get_tags_from_request_body(call["request_body"]),
    )


def populate_call(input_value: Json) -> Json:
    call = POPULATE.validate_python(input_value)
    request_data = dict(call["request_data"])
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L654
        "dict[str, Json]",
        populate_request_with_path_params(request_data=request_data, request=build_request(call)),
    )


RUNNERS: dict[str, Callable[[Json], Json]] = {
    "resolve_inference_model": resolve_call,
    "reject_url_valued_destination": reject_call,
    "reject_url_valued_destinations": reject_data_call,
    "tags_from_request_body": tags_call,
    "populate_request_parameters": populate_call,
}


def run(input_value: Json) -> Json:
    calls = CALLS.validate_python(input_value)
    results: list[Json] = []
    for call in calls["calls"]:
        function = ENVELOPE.validate_python(call)["function"]
        results.append(RUNNERS[function](call))
    return results
