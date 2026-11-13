import json
from collections.abc import Callable, Sequence
from contextlib import suppress
from typing import Literal, Protocol, TypedDict, cast

import httpx
from litellm.llms.black_forest_labs.common_utils import assert_bfl_polling_url
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


class GenerationConfig(Protocol):
    get_supported_openai_params: Callable[[str], Sequence[str]]
    map_openai_params: Callable[
        [dict[str, JsonValue], dict[str, JsonValue], str, bool],
        dict[str, JsonValue],
    ]
    validate_environment: Callable[
        [
            dict[str, str],
            str,
            list[JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            str | None,
        ],
        dict[str, str],
    ]
    get_complete_url: Callable[
        [str | None, str | None, str, dict[str, JsonValue], dict[str, JsonValue]],
        str,
    ]
    # ast-grep-ignore: no-long-names
    transform_image_generation_request: Callable[
        [str, str, dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
        dict[str, JsonValue],
    ]
    # ast-grep-ignore: no-long-names
    transform_image_generation_response: Callable[
        [
            str,
            httpx.Response,
            ImageResponse,
            JsonValue,
            dict[str, JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            None,
        ],
        ImageResponse,
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


def config_for(model: str) -> GenerationConfig:
    config: GenerationConfig = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/black_forest_labs/image_generation/transformation.py#L313-L322
        GenerationConfig,
        get_black_forest_labs_image_generation_config(model),
    )
    return config


def mapped_body(config: GenerationConfig, call: MappingCall, request: MappingRequest) -> JsonValue:
    mapped = config.map_openai_params(
        request["params"],
        request["optional_params"],
        call["model"],
        call["drop_params"],
    )
    body = config.transform_image_generation_request(
        call["model"], call["prompt"], mapped, {}, {}
    )
    answer: dict[str, JsonValue] = {"params": mapped, "body": body}
    return JSON.validate_python(answer)


def mapping(call: MappingCall) -> JsonValue:
    config = config_for(call["model"])
    supported = JSON.validate_python(config.get_supported_openai_params(call["model"]))
    results = [mapped_body(config, call, request) for request in call["requests"]]
    answer: dict[str, JsonValue] = {"supported": supported, "results": results}
    return JSON.validate_python(answer)


def credential_outcome(
    config: GenerationConfig, call: CredentialsCall, request: CredentialRequest
) -> JsonValue:
    headers = config.validate_environment(
        {},
        call["model"],
        [],
        {},
        {},
        request["api_key"],
    )
    url = config.get_complete_url(
        request["api_base"], request["api_key"], call["model"], {}, {}
    )
    lowered_headers: dict[str, str] = {
        name.lower(): value for name, value in headers.items()
    }
    lowered = JSON.validate_python(lowered_headers)
    answer: dict[str, JsonValue] = {"headers": lowered, "url": url}
    return JSON.validate_python(answer)


def credentials(call: CredentialsCall) -> JsonValue:
    config = config_for(call["model"])
    outcomes = [credential_outcome(config, call, request) for request in call["requests"]]
    answer: dict[str, JsonValue] = {"outcomes": outcomes}
    return JSON.validate_python(answer)


def appended_reply(config: GenerationConfig, reply: ReplyCall) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "created": 0,
        "data": JSON.validate_python(reply["existing"]),
    }
    model_response = TypeAdapter(ImageResponse).validate_python(payload)
    request = httpx.Request("POST", "https://api.bfl.ai")
    raw_response = httpx.Response(200, content=json.dumps(reply["body"]).encode(), request=request)
    result = config.transform_image_generation_response(
        "",
        raw_response,
        model_response,
        None,
        {},
        {},
        {},
        None,
    )
    return JSON.validate_python(IMAGES.validate_json(result.model_dump_json()))


def append_results(call: AppendCall) -> JsonValue:
    _ = call["now"]
    config = config_for("")
    responses = [appended_reply(config, reply) for reply in call["replies"]]
    answer: dict[str, JsonValue] = {"responses": responses}
    return JSON.validate_python(answer)


def poll_outcome(url: str) -> JsonValue:
    accepted = False
    with suppress(Exception):
        assert_bfl_polling_url(url)
        accepted = True
    answer: dict[str, JsonValue] = {"url": url, "accepted": accepted}
    return JSON.validate_python(answer)


def poll_host(call: PollCall) -> JsonValue:
    outcomes = [poll_outcome(url) for url in call["urls"]]
    answer: dict[str, JsonValue] = {"urls": outcomes}
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "mapping":
            return mapping(input_value)
        case "credentials":
            return credentials(input_value)
        case "append_results":
            return append_results(input_value)
        case "poll_host":
            return poll_host(input_value)
