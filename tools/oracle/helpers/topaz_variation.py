import asyncio
import base64
import os
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Final, Literal, NotRequired, TypedDict

import httpx
from aiohttp import ClientResponse
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.topaz.common_utils import TopazModelInfo
from litellm.llms.topaz.image_variations.transformation import TopazImageVariationConfig
from litellm.types.utils import FileTypes, ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

MODEL: Final = "topaz/CGI"
EMPTY: Final[dict[str, JsonValue]] = {}
CONFIG: Final = TopazImageVariationConfig()
INFO: Final = TopazModelInfo()
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
Prepared = tuple[str, bytes | None, str, dict[str, JsonValue]]


class FieldsView(TypedDict):
    files: dict[str, Prepared]
    data: dict[str, JsonValue]


class ResponseView(TypedDict):
    created: int
    data: list[dict[str, JsonValue]]
    usage: dict[str, JsonValue]


PREPARED: Final = TypeAdapter(Prepared)
FIELDS: Final = TypeAdapter(FieldsView)
RESPONSE: Final = TypeAdapter(ResponseView)


class ModelsCall(TypedDict):
    function: Literal["models"]


class BaseModelCall(TypedDict):
    function: Literal["base_model"]
    model: str


class SupportedCall(TypedDict):
    function: Literal["supported"]


class CredentialCall(TypedDict):
    function: Literal["credential"]
    field: Literal["api_key", "api_base"]
    value: str | None
    environment: dict[str, str]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    api_key: str | None
    headers: dict[str, str]


class UrlCall(TypedDict):
    function: Literal["url"]
    api_base: str | None


class MapCall(TypedDict):
    function: Literal["map"]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class FileSpec(TypedDict):
    kind: Literal["bytes", "tuple2", "tuple3", "tuple4", "other"]
    content: str
    filename: str | None
    content_type: str | None
    headers: dict[str, str]


class UploadCall(TypedDict):
    function: Literal["upload"]
    file: FileSpec
    optional_params: NotRequired[dict[str, JsonValue] | None]


class SyncCall(TypedDict):
    function: Literal["response_sync"]
    body: list[int]
    elapsed_ms: float
    created: int


class AsyncCall(TypedDict):
    function: Literal["response_async"]
    body: list[int]
    response_ms: float
    created: int


Call = (
    ModelsCall | BaseModelCall | SupportedCall | CredentialCall | EnvironmentCall | UrlCall
    | MapCall | UploadCall | SyncCall | AsyncCall
)


class Input(TypedDict):
    calls: list[Call]


class ScriptedLog(Logging):
    measures: int = 0
    measured: float = 0.0

    @override
    def get_response_ms(self) -> float:
        self.measures += 1
        return self.measured


class BodyResponse(ClientResponse):
    body: bytes

    @override
    async def read(self) -> bytes:
        return self.body


def scripted_log() -> ScriptedLog:
    return ScriptedLog(
        model=MODEL, messages=None, stream=False, call_type="image_variation",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="topaz-variation-call", function_id="topaz-variation-function",
    )


def apply_environment(environment: dict[str, str]) -> None:
    os.environ.pop("TOPAZ_API_KEY", None)
    os.environ.pop("TOPAZ_API_BASE", None)
    os.environ.update(environment)


def file_value(spec: FileSpec) -> FileTypes:
    if spec["kind"] == "bytes":
        return spec["content"].encode()
    if spec["kind"] == "tuple2":
        return (spec["filename"], spec["content"].encode())
    if spec["kind"] == "tuple3":
        return (spec["filename"], spec["content"].encode(), spec["content_type"])
    if spec["kind"] == "tuple4":
        return (
            spec["filename"], spec["content"].encode(),
            spec["content_type"], spec["headers"],
        )
    return Path("image.png")


def prepared_json(prepared: Prepared) -> JsonValue:
    content = prepared[1]
    return {
        "filename": prepared[0],
        "content": None if content is None else base64.b64encode(content).decode(),
        "content_type": prepared[2],
        "headers": prepared[3],
    }


def response_json(response: ImageResponse) -> JsonValue:
    view = RESPONSE.validate_json(response.model_dump_json())
    images: list[JsonValue] = [
        {
            "b64_json": image.get("b64_json"), "url": image.get("url"),
            "revised_prompt": image.get("revised_prompt"),
        }
        for image in view["data"]
    ]
    return {"created": view["created"], "data": images, "usage": view["usage"]}


def timing_source(log: ScriptedLog) -> str:
    if log.measures:
        return "measured"
    return "elapsed"


def catalogue_answer(call: ModelsCall | BaseModelCall | SupportedCall) -> JsonValue:
    match call["function"]:
        case "models":
            return [str(name) for name in INFO.get_models()]
        case "base_model":
            return INFO.get_base_model(call["model"])
        case "supported":
            return [str(name) for name in CONFIG.get_supported_openai_params(MODEL)]


def credential_answer(call: CredentialCall | EnvironmentCall) -> JsonValue:
    match call["function"]:
        case "credential":
            apply_environment(call["environment"])
            if call["field"] == "api_key":
                return {"api_key": INFO.get_api_key(call["value"])}
            return {"api_base": INFO.get_api_base(call["value"])}
        case "environment":
            try:
                return {"headers": OBJECT.validate_python(CONFIG.validate_environment(
                    call["headers"], MODEL, [], EMPTY, EMPTY, call["api_key"], None
                ))}
            except ValueError:
                return {"missing_key": True}


def mapping_answer(call: UrlCall | MapCall) -> JsonValue:
    match call["function"]:
        case "url":
            return {"url": CONFIG.get_complete_url(call["api_base"], None, MODEL, EMPTY, EMPTY)}
        case "map":
            try:
                return {"mapped": OBJECT.validate_python(CONFIG.map_openai_params(
                    call["non_default_params"], call["optional_params"],
                    MODEL, call["drop_params"],
                ))}
            except AssertionError:
                return {"invalid": True}


def upload_answer(call: UploadCall) -> JsonValue:
    file = file_value(call["file"])
    optional_params = call.get("optional_params")
    if optional_params is None:
        return prepared_json(PREPARED.validate_python(CONFIG.prepare_file_tuple(file)))
    fields = FIELDS.validate_python(
        CONFIG.transform_request_image_variation(MODEL, file, optional_params, EMPTY))
    prepared = prepared_json(fields["files"]["image"])
    return {"files": {"image": prepared}, "data": fields["data"]}


async def measure_async(call: AsyncCall) -> JsonValue:
    response = BodyResponse.__new__(BodyResponse)
    response.body = bytes(call["body"])
    log = scripted_log()
    log.measured = call["response_ms"]
    finished = await CONFIG.async_transform_response_image_variation(
        MODEL, response, ImageResponse(created=call["created"]),
        log, EMPTY, b"pixels", EMPTY, EMPTY, None,
    )
    return {
        "response": response_json(finished),
        "response_ms": log.measured,
        "timing_source": timing_source(log),
    }


def reply_answer(call: SyncCall | AsyncCall) -> JsonValue:
    match call["function"]:
        case "response_sync":
            response = httpx.Response(200, content=bytes(call["body"]))
            response.elapsed = timedelta(milliseconds=call["elapsed_ms"])
            log = scripted_log()
            finished = CONFIG.transform_response_image_variation(
                MODEL, response, ImageResponse(created=call["created"]),
                log, EMPTY, b"pixels", EMPTY, EMPTY, None,
            )
            return {
                "response": response_json(finished),
                "response_ms": response.elapsed.total_seconds() * 1000,
                "timing_source": timing_source(log),
            }
        case "response_async":
            return asyncio.run(measure_async(call))


def call_value(call: Call) -> JsonValue:
    match call["function"]:
        case "models" | "base_model" | "supported":
            return catalogue_answer(call)
        case "credential" | "environment":
            return credential_answer(call)
        case "url" | "map":
            return mapping_answer(call)
        case "upload":
            return upload_answer(call)
        case "response_sync" | "response_async":
            return reply_answer(call)


def run(input_value: Input) -> JsonValue:
    return [
        call_value(call)
        for call in input_value["calls"]
    ]
