import json
from collections.abc import Callable
from typing import Literal, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.llms.black_forest_labs.common_utils import BlackForestLabsError, assert_bfl_polling_url
from litellm.llms.black_forest_labs.image_generation.transformation import (
    get_black_forest_labs_image_generation_config,
)
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ImageDatumFields(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class ImageResponseFields(TypedDict):
    created: int
    data: list[ImageDatumFields]


IMAGES: TypeAdapter[ImageResponseFields] = TypeAdapter(ImageResponseFields)

Calls: TypeAlias = tuple[
    Callable[[str], list[str]],
    Callable[..., dict[str, JsonValue]],
    Callable[..., dict[str, str]],
    Callable[..., str],
    Callable[..., dict[str, JsonValue]],
    Callable[..., ImageResponse],
]


class MappingRequest(TypedDict):
    params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class MappingCall(TypedDict):
    operation: Literal["mapping"]
    model: str
    prompt: str
    drop_params: bool
    requests: list[MappingRequest]


class CredentialRequest(TypedDict):
    api_key: str | None
    api_base: str | None


class CredentialsCall(TypedDict):
    operation: Literal["credentials"]
    model: str
    environment: dict[str, str]
    requests: list[CredentialRequest]


class ReplyCall(TypedDict):
    existing: list[dict[str, JsonValue]]
    body: dict[str, JsonValue]


class AppendCall(TypedDict):
    operation: Literal["append_results"]
    now: int
    replies: list[ReplyCall]


class PollCall(TypedDict):
    operation: Literal["poll_host"]
    urls: list[str]


Input = MappingCall | CredentialsCall | AppendCall | PollCall


def calls_for(model: str) -> Calls:
    config = get_black_forest_labs_image_generation_config(model)
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/black_forest_labs/image_generation/transformation.py#L237-L294
        Calls,
        (
            config.get_supported_openai_params,
            config.map_openai_params,
            config.validate_environment,
            config.get_complete_url,
            config.transform_image_generation_request,
            config.transform_image_generation_response,
        ),
    )


def mapped_body(call: MappingCall, request: MappingRequest) -> JsonValue:
    _supported, map_params, _validate, _url, body_of, _response = calls_for(call["model"])
    mapped = map_params(
        request["params"],
        request["optional_params"],
        call["model"],
        call["drop_params"],
    )
    body = body_of(call["model"], call["prompt"], mapped, {}, {})
    return JSON.validate_python({"params": mapped, "body": body})


def mapping(call: MappingCall) -> JsonValue:
    supported, _map, _validate, _url, _body, _response = calls_for(call["model"])
    results = [mapped_body(call, request) for request in call["requests"]]
    return JSON.validate_python({"supported": supported(call["model"]), "results": results})


def credential_outcome(call: CredentialsCall, request: CredentialRequest) -> JsonValue:
    _supported, _map, validate, complete_url, _body, _response = calls_for(call["model"])
    headers = validate(
        {},
        call["model"],
        [],
        {},
        {},
        api_key=request["api_key"],
        api_base=request["api_base"],
    )
    url = complete_url(request["api_base"], request["api_key"], call["model"], {}, {})
    lowered = {name.lower(): value for name, value in headers.items()}
    return JSON.validate_python({"headers": lowered, "url": url})


def credentials(call: CredentialsCall) -> JsonValue:
    outcomes = [credential_outcome(call, request) for request in call["requests"]]
    return JSON.validate_python({"outcomes": outcomes})


def appended_reply(reply: ReplyCall) -> JsonValue:
    _supported, _map, _validate, _url, _body, response_of = calls_for("")
    model_response = ImageResponse.model_validate({"created": 0, "data": reply["existing"]})
    request = httpx.Request("POST", "https://api.bfl.ai")
    raw_response = httpx.Response(200, content=json.dumps(reply["body"]).encode(), request=request)
    result = response_of(
        model="",
        raw_response=raw_response,
        model_response=model_response,
        logging_obj=None,
        request_data={},
        optional_params={},
        litellm_params={},
        encoding=None,
        api_key=None,
    )
    return JSON.validate_python(IMAGES.validate_json(result.model_dump_json()))


def append_results(call: AppendCall) -> JsonValue:
    _ = call["now"]
    responses = [appended_reply(reply) for reply in call["replies"]]
    return JSON.validate_python({"responses": responses})


def poll_outcome(url: str) -> JsonValue:
    try:
        assert_bfl_polling_url(url)
    except BlackForestLabsError as error:
        return JSON.validate_python({"url": url, "accepted": False, "status": error.status_code})
    return JSON.validate_python({"url": url, "accepted": True, "status": None})


def poll_host(call: PollCall) -> JsonValue:
    outcomes = [poll_outcome(url) for url in call["urls"]]
    return JSON.validate_python({"urls": outcomes})


def run(input_value: Input) -> JsonValue:
    _ = litellm
    match input_value["operation"]:
        case "mapping":
            return mapping(input_value)
        case "credentials":
            return credentials(input_value)
        case "append_results":
            return append_results(input_value)
        case "poll_host":
            return poll_host(input_value)
