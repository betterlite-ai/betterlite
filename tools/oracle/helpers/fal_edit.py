import datetime as dt
import io
import json
import os
import tempfile
from collections.abc import Sized
from pathlib import Path
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.fal_ai.image_edit import get_fal_ai_image_edit_config
from litellm.llms.fal_ai.image_edit.transformation import to_data_url
from litellm.types.images.main import ImageEditOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
GENERIC_MODEL = "openai/gpt-image-2/edit"
TUPLE_NAME = "fal-edit-tuple-fixture.bin"

Prepared: TypeAlias = bytes | Path | io.BytesIO | tuple[str, bytes]


class EditConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        image_edit_optional_params: ImageEditOptionalRequestParams,
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        model: str,
        api_base: str | None,
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def transform_image_edit_request(
        self,
        model: str,
        prompt: str | None,
        image: Prepared | None,
        image_edit_optional_request_params: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
        /,
    ) -> tuple[dict[str, JsonValue], Sized]: ...

    def transform_image_edit_response(
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
    ) -> ImageResponse: ...


class BytesInput(TypedDict):
    kind: Literal["bytes"]
    content: list[int]


class PathInput(TypedDict):
    kind: Literal["path"]
    name: str
    content: list[int]


class ReaderInput(TypedDict):
    kind: Literal["reader"]
    content: list[int]
    position: int


class TupleInput(TypedDict):
    kind: Literal["tuple"]
    content: list[int]


class ValueInput(TypedDict):
    kind: Literal["value"]
    value: JsonValue


Source: TypeAlias = BytesInput | PathInput | ReaderInput | TupleInput | ValueInput
PrepareImage: TypeAlias = BytesInput | PathInput | ReaderInput | TupleInput


class ImageFields(TypedDict, total=False):
    url: str | None
    b64_json: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class ReplyInput(TypedDict):
    status: int
    headers: NotRequired[dict[str, str]]
    body: str


class DataUrlCall(TypedDict):
    operation: Literal["to_data_url"]
    sources: list[Source]


class PrepareCall(TypedDict):
    operation: Literal["prepare"]
    model: str
    prompt: NotRequired[str | None]
    image: NotRequired[PrepareImage]
    params: dict[str, JsonValue]


class MapCall(TypedDict):
    operation: Literal["map"]
    model: str
    options: list[ImageEditOptionalRequestParams]


class UrlCall(TypedDict):
    operation: Literal["url"]
    environment: dict[str, str]
    api_bases: list[str | None]
    models: list[str]


class ReplyCall(TypedDict):
    operation: Literal["reply"]
    replies: list[ReplyInput]


Input: TypeAlias = DataUrlCall | PrepareCall | MapCall | UrlCall | ReplyCall


def apply_environment(environment: dict[str, str]) -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def fixture_path(name: str, content: list[int]) -> Path:
    path = Path(tempfile.gettempdir()) / name
    path.write_bytes(bytes(content))
    return path


def error_message(error: ValueError) -> JsonValue:
    return {"error": {"status": None, "message": str(error)}}


def data_url_outcome(image: Prepared | JsonValue) -> JsonValue:
    try:
        url = to_data_url(image)
    except ValueError as error:
        return error_message(error)
    return {"data_url": url, "position": None}


def reader_outcome(source: ReaderInput) -> JsonValue:
    reader = io.BytesIO(bytes(source["content"]))
    reader.seek(source["position"])
    try:
        url = to_data_url(reader)
    except ValueError as error:
        return error_message(error)
    return {"data_url": url, "position": reader.tell()}


def source_outcome(source: Source) -> JsonValue:
    if source["kind"] == "reader":
        return reader_outcome(source)
    if source["kind"] == "tuple":
        return data_url_outcome((TUPLE_NAME, bytes(source["content"])))
    if source["kind"] == "value":
        return data_url_outcome(source["value"])
    if source["kind"] == "path":
        return data_url_outcome(fixture_path(source["name"], source["content"]))
    return data_url_outcome(bytes(source["content"]))


def prepare_body(
    call: PrepareCall, image: Prepared | None, cursor: io.BytesIO | None
) -> JsonValue:
    config: EditConfig = get_fal_ai_image_edit_config(call["model"])
    try:
        body, files = config.transform_image_edit_request(
            call["model"],
            call.get("prompt"),
            image,
            call["params"],
            GenericLiteLLMParams(),
            {},
        )
    except ValueError as error:
        return error_message(error)
    return {
        "body": PARAMS.validate_json(json.dumps(body)),
        "files": len(files),
        "position": None if cursor is None else cursor.tell(),
    }


def prepare_outcome(call: PrepareCall) -> JsonValue:
    image = call.get("image")
    if image is None:
        return prepare_body(call, None, None)
    if image["kind"] == "reader":
        reader = io.BytesIO(bytes(image["content"]))
        reader.seek(image["position"])
        return prepare_body(call, reader, reader)
    if image["kind"] == "bytes":
        return prepare_body(call, bytes(image["content"]), None)
    if image["kind"] == "path":
        return prepare_body(call, fixture_path(image["name"], image["content"]), None)
    return prepare_body(call, (TUPLE_NAME, bytes(image["content"])), None)


def mapped_params(
    config: EditConfig, model: str, options: ImageEditOptionalRequestParams
) -> JsonValue:
    mapped = config.map_openai_params(options, model, drop_params=False)
    return {"params": PARAMS.validate_python(mapped)}


def map_outcome(call: MapCall) -> JsonValue:
    config: EditConfig = get_fal_ai_image_edit_config(call["model"])
    supported: list[str] = config.get_supported_openai_params(call["model"])
    cases: list[JsonValue] = [
        mapped_params(config, call["model"], options) for options in call["options"]
    ]
    return {"supported": [JSON.validate_python(name) for name in supported], "cases": cases}


def url_entry(api_base: str | None, model: str) -> JsonValue:
    config: EditConfig = get_fal_ai_image_edit_config(model)
    url = config.get_complete_url(model=model, api_base=api_base, litellm_params={})
    return {"model": model, "api_base": api_base, "url": url}


def url_outcome(call: UrlCall) -> JsonValue:
    apply_environment(call["environment"])
    urls: list[JsonValue] = [
        url_entry(api_base, model)
        for api_base in call["api_bases"]
        for model in call["models"]
    ]
    return {"urls": urls}


def logging_object() -> Logging:
    return Logging(
        model=GENERIC_MODEL,
        messages=None,
        stream=False,
        call_type="image_edit",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="b1526",
        function_id="b1526",
    )


def reply_outcome(call: ReplyInput) -> JsonValue:
    response = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode()
    )
    config: EditConfig = get_fal_ai_image_edit_config(GENERIC_MODEL)
    try:
        reply = config.transform_image_edit_response(
            model=GENERIC_MODEL, raw_response=response, logging_obj=logging_object()
        )
    except BaseLLMException as error:
        return {"error": {"status": error.status_code}}
    return {
        "images": [
            JSON.validate_python(TypeAdapter(ImageFields).validate_json(image.model_dump_json()))
            for image in reply.data
        ]
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "to_data_url":
        return {"sources": [source_outcome(source) for source in input_value["sources"]]}
    if input_value["operation"] == "prepare":
        return prepare_outcome(input_value)
    if input_value["operation"] == "map":
        return map_outcome(input_value)
    if input_value["operation"] == "url":
        return url_outcome(input_value)
    return {"replies": [reply_outcome(reply) for reply in input_value["replies"]]}
