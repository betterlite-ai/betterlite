import datetime as dt
import io
import os
import tempfile
from pathlib import Path
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.fal_ai.image_edit import get_fal_ai_image_edit_config
from litellm.llms.fal_ai.image_edit.transformation import to_data_url
from litellm.types.images.main import ImageEditOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
GENERIC_MODEL = "openai/gpt-image-2/edit"
TUPLE_NAME = "fal-edit-tuple-fixture.bin"
PROVIDER = "fal_ai"
ORACLE_API_KEY = "fal-edit-oracle"

Prepared: TypeAlias = str | bytes | Path | io.BytesIO | tuple[str, bytes]
Options: TypeAlias = dict[str, JsonValue | list[Prepared | None]]
EditParams: TypeAlias = ImageEditOptionalRequestParams
JsonObject: TypeAlias = dict[str, JsonValue]


class EditConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self, image_edit_optional_params: EditParams, model: str, *, drop_params: bool
    ) -> JsonObject: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: JsonObject
    ) -> str: ...

    def transform_image_edit_response(
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> ImageResponse: ...


class ContentInput(TypedDict):
    kind: Literal["bytes", "tuple"]
    content: list[int]


class PathInput(TypedDict):
    kind: Literal["path"]
    name: str
    content: list[int]


class ReaderInput(TypedDict):
    kind: Literal["reader"]
    content: list[int]
    position: int


class TextInput(TypedDict):
    kind: Literal["text"]
    text: str


class ValueInput(TypedDict):
    kind: Literal["value"]
    value: JsonValue


class PreparedImage(TypedDict):
    image: Prepared | None
    position: int | None


Source: TypeAlias = ContentInput | PathInput | ReaderInput | ValueInput
PrepareImage: TypeAlias = ContentInput | PathInput | ReaderInput | TextInput


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
    images: list[PrepareImage | None]
    mask: list[PrepareImage | None]
    params: dict[str, JsonValue]


class MapCall(TypedDict):
    operation: Literal["map"]
    model: str
    options: list[EditParams]


class UrlCall(TypedDict):
    operation: Literal["url"]
    environment: dict[str, str]
    api_bases: list[str | None]
    models: list[str]


class ReplyCall(TypedDict):
    operation: Literal["reply"]
    replies: list[ReplyInput]


Input: TypeAlias = DataUrlCall | PrepareCall | MapCall | UrlCall | ReplyCall


class Recorder:
    def __init__(self) -> None:
        self.body: bytes | None = None

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.body = request.read()
        return httpx.Response(200, headers={"x-request-id": "req-1"}, content=b'{"images": []}')


class ProbeLogging(Logging):
    @override
    def __init__(self) -> None:
        pass

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args, params


def fixture_path(name: str, content: list[int]) -> Path:
    path = Path(tempfile.gettempdir()) / name
    path.write_bytes(bytes(content))
    return path


def source_outcome(source: Source) -> JsonValue:
    if source["kind"] == "reader":
        reader = io.BytesIO(bytes(source["content"]))
        reader.seek(source["position"])
        try:
            url = to_data_url(reader)
        except ValueError as error:
            return {"error": {"status": None, "message": str(error)}}
        return {"data_url": url, "position": reader.tell()}
    try:
        if source["kind"] == "tuple":
            url = to_data_url((TUPLE_NAME, bytes(source["content"])))
        elif source["kind"] == "value":
            url = to_data_url(source["value"])
        elif source["kind"] == "path":
            url = to_data_url(fixture_path(source["name"], source["content"]))
        else:
            url = to_data_url(bytes(source["content"]))
    except ValueError as error:
        return {"error": {"status": None, "message": str(error)}}
    return {"data_url": url, "position": None}


def prepared_image(entry: PrepareImage | None) -> PreparedImage:
    if entry is None:
        return {"image": None, "position": None}
    if entry["kind"] == "text":
        return {"image": entry["text"], "position": None}
    if entry["kind"] == "reader":
        reader = io.BytesIO(bytes(entry["content"]))
        reader.seek(entry["position"])
        return {"image": reader, "position": entry["position"]}
    if entry["kind"] == "path":
        return {"image": fixture_path(entry["name"], entry["content"]), "position": None}
    if entry["kind"] == "tuple":
        return {"image": (TUPLE_NAME, bytes(entry["content"])), "position": None}
    return {"image": bytes(entry["content"]), "position": None}


def edit_body(call: PrepareCall, images: list[Prepared | None], options: Options) -> JsonObject:
    recorder = Recorder()
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder)))
    config: BaseImageEditConfig = get_fal_ai_image_edit_config(call["model"])
    BaseLLMHTTPHandler().image_edit_handler(
        call["model"], images, call.get("prompt"), config, options, PROVIDER,
        GenericLiteLLMParams(api_key=ORACLE_API_KEY), ProbeLogging(), 30.0, None, None, client,
    )
    if recorder.body is None:
        raise ValueError("the edit handler sent no request")
    return PARAMS.validate_json(recorder.body)


def prepare_outcome(call: PrepareCall) -> JsonValue:
    prepared = [prepared_image(entry) for entry in call["images"]]
    mask_prepared = [prepared_image(entry) for entry in call["mask"]]
    images = [item["image"] for item in prepared]
    mask = [item["image"] for item in mask_prepared]
    options: Options = {**call["params"]}
    if mask:
        options["mask"] = mask
    try:
        body = edit_body(call, images, options)
    except ValueError as error:
        return {"error": {"status": None, "message": str(error)}}
    return JSON.validate_python(
        {
            "body": body,
            "positions": [item["position"] for item in prepared],
            "mask_positions": [item["position"] for item in mask_prepared],
        }
    )


def map_outcome(call: MapCall) -> JsonValue:
    config: EditConfig = get_fal_ai_image_edit_config(call["model"])
    supported: list[str] = config.get_supported_openai_params(call["model"])
    cases: list[JsonValue] = []
    for options in call["options"]:
        mapped = config.map_openai_params(options, call["model"], drop_params=False)
        cases.append({"params": PARAMS.validate_python(mapped)})
    return {"supported": [JSON.validate_python(name) for name in supported], "cases": cases}


def url_outcome(call: UrlCall) -> JsonValue:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in call["environment"].items():
        os.environ[name] = value
    urls: list[JsonValue] = []
    for api_base in call["api_bases"]:
        for model in call["models"]:
            config: EditConfig = get_fal_ai_image_edit_config(model)
            url = config.get_complete_url(model=model, api_base=api_base, litellm_params={})
            urls.append({"model": model, "api_base": api_base, "url": url})
    return {"urls": urls}


def logging_object() -> Logging:
    return Logging(
        model=GENERIC_MODEL, messages=None, stream=False, call_type="image_edit",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC), litellm_call_id="b1526",
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
    dumped = PARAMS.validate_json(reply.model_dump_json())
    dumped.pop("created", None)
    return JSON.validate_python(dumped)


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
