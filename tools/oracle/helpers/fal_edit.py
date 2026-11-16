import io
import os
import tempfile
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.fal_ai.image_edit import get_fal_ai_image_edit_config
from litellm.llms.fal_ai.image_edit.transformation import to_data_url
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
GENERIC_MODEL = "openai/gpt-image-2/edit"
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

Prepared: TypeAlias = str | bytes | Path | io.BytesIO | tuple[str, bytes]
Options: TypeAlias = dict[str, JsonValue | list[Prepared | None]]
JsonObject: TypeAlias = dict[str, JsonValue]


class EditParams(TypedDict, total=False):
    background: Literal["transparent", "opaque", "auto"] | None
    input_fidelity: Literal["high", "low"] | None
    mask: str | None
    n: int | None
    quality: Literal["high", "medium", "low", "standard", "auto"] | None
    response_format: Literal["url", "b64_json"] | None
    size: str | None
    user: str | None
    imageConfig: dict[str, JsonValue] | None


class ErrorRecord(BaseModel):
    status_code: int


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


class ConfigSelector(Protocol):
    def __call__(self, model: str) -> EditConfig: ...


class EditHandler(Protocol):
    @property
    def image_edit_handler(self) -> Callable[
        [str, list[Prepared | None], str | None, BaseImageEditConfig, Options, str,
         GenericLiteLLMParams, Logging, float, None, None, HTTPHandler],
        ImageResponse | Awaitable[ImageResponse],
    ]: ...


SELECT_CONFIG: ConfigSelector = get_fal_ai_image_edit_config
HANDLER: EditHandler = BaseLLMHTTPHandler()


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
    reader: io.BytesIO | None


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


def prepared_image(entry: PrepareImage | None) -> PreparedImage:
    if entry is None:
        return {"image": None, "reader": None}
    if entry["kind"] == "text":
        return {"image": entry["text"], "reader": None}
    if entry["kind"] == "reader":
        reader = io.BytesIO(bytes(entry["content"]))
        reader.seek(entry["position"])
        return {"image": reader, "reader": reader}
    if entry["kind"] == "path":
        path = Path(tempfile.gettempdir()) / entry["name"]
        path.write_bytes(bytes(entry["content"]))
        return {"image": path, "reader": None}
    if entry["kind"] == "tuple":
        return {"image": ("fal-edit-tuple-fixture.bin", bytes(entry["content"])), "reader": None}
    return {"image": bytes(entry["content"]), "reader": None}


def reader_positions(items: list[PreparedImage]) -> list[JsonValue]:
    positions: list[JsonValue] = []
    for item in items:
        reader = item["reader"]
        positions.append(None if reader is None else reader.tell())
    return positions


def source_outcome(source: Source) -> JsonValue:
    image: Prepared | JsonValue
    reader: io.BytesIO | None = None
    if source["kind"] == "value":
        image = source["value"]
    else:
        prepared = prepared_image(source)
        image = prepared["image"]
        reader = prepared["reader"]
    try:
        url = to_data_url(image)
    except ValueError as error:
        return {"error": {"status": None, "message": str(error)}}
    return {"data_url": url, "position": None if reader is None else reader.tell()}


def edit_body(call: PrepareCall, images: list[Prepared | None], options: Options) -> JsonObject:
    recorder = Recorder()
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder)))
    config = get_fal_ai_image_edit_config(call["model"])
    HANDLER.image_edit_handler(
        call["model"], images, call.get("prompt"), config, options, "fal_ai",
        GenericLiteLLMParams(api_key="fal-edit-oracle"), ProbeLogging(), 30.0, None, None,
        client,
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
    return {
        "body": body,
        "positions": reader_positions(prepared),
        "mask_positions": reader_positions(mask_prepared),
    }


def map_outcome(call: MapCall) -> JsonValue:
    config = SELECT_CONFIG(call["model"])
    supported: list[JsonValue] = [*config.get_supported_openai_params(call["model"])]
    cases: list[JsonValue] = [
        {"params": config.map_openai_params(options, call["model"], drop_params=False)}
        for options in call["options"]
    ]
    return {"supported": supported, "cases": cases}


def url_outcome(call: UrlCall) -> JsonValue:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in call["environment"].items():
        os.environ[name] = value
    urls: list[JsonValue] = []
    for api_base in call["api_bases"]:
        for model in call["models"]:
            config = SELECT_CONFIG(model)
            url = config.get_complete_url(model=model, api_base=api_base, litellm_params={})
            urls.append({"model": model, "api_base": api_base, "url": url})
    return {"urls": urls}


def reply_outcome(call: ReplyInput) -> JsonValue:
    response = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode()
    )
    try:
        reply = SELECT_CONFIG(GENERIC_MODEL).transform_image_edit_response(
            model=GENERIC_MODEL, raw_response=response, logging_obj=ProbeLogging()
        )
    except PROVIDER_ERROR as error:
        record = ErrorRecord.model_validate(error, from_attributes=True)
        return {"error": {"status": record.status_code}}
    dumped = PARAMS.validate_json(reply.model_dump_json())
    dumped.pop("created", None)
    return dumped


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
