import asyncio
import base64
import os
from collections.abc import Awaitable, Callable, Sequence
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
from aiohttp import ClientResponse
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.topaz.common_utils import TopazModelInfo
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import FileTypes, ImageResponse, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

MODEL: Final = "topaz/CGI"
EMPTY: Final[dict[str, JsonValue]] = {}
INFO: Final = TopazModelInfo()
Prepared = tuple[str, bytes | None, str, dict[str, JsonValue]]
Obj = dict[str, JsonValue]


class NoMessages(list[AllMessageValues]): ...


class FieldsView(TypedDict):
    files: dict[str, Prepared]
    data: dict[str, JsonValue]


class ResponseView(TypedDict):
    created: int
    data: list[dict[str, JsonValue]]
    usage: dict[str, JsonValue]


RESPONSE: Final = TypeAdapter(ResponseView)

EnvTransform = Callable[[dict[str, str], str, NoMessages, Obj, Obj, str | None, str | None], Obj]
UrlTransform = Callable[[str | None, str | None, str, Obj, Obj], str]
ReplyTransform = Callable[
    [str | None, httpx.Response, ImageResponse, Logging, Obj, FileTypes, Obj, Obj, None],
    ImageResponse,
]
AsyncReplyFn = Callable[
    [str | None, ClientResponse, ImageResponse, Logging, Obj, FileTypes, Obj, Obj, None],
    Awaitable[ImageResponse],
]


@runtime_checkable
class TopazConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...

    @property
    def get_complete_url(self) -> UrlTransform: ...

    def map_openai_params(
        self, non_default_params: Obj, optional_params: Obj, model: str, *, drop_params: bool,
    ) -> Obj: ...

    def prepare_file_tuple(self, file_data: FileTypes) -> Prepared: ...

    def transform_request_image_variation(  # ast-grep-ignore: no-long-names
        self, model: str | None, image: FileTypes, optional_params: Obj, headers: Obj,
    ) -> FieldsView: ...

    @property
    def validate_environment(self) -> EnvTransform: ...

    @property
    def transform_response_image_variation(self) -> ReplyTransform: ...  # ast-grep-ignore

    @property
    def async_transform_response_image_variation(self) -> AsyncReplyFn: ...  # ast-grep-ignore


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: TopazConfig


def topaz_config() -> TopazConfig:
    found = ProviderConfigManager.get_provider_image_variation_config(MODEL, LlmProviders.TOPAZ)
    if found is None:
        raise ValueError(f"no image variation config is registered for {MODEL}")
    return ConfigBoundary.model_validate({"config": found}).config  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L9400-L9408


CONFIG: Final = topaz_config()


class FileSpec(BaseModel):
    kind: Literal["bytes", "tuple2", "tuple3", "tuple4", "other"]
    content: str = ""
    filename: str | None = None
    content_type: str | None = None
    headers: dict[str, str] = {}


class Call(BaseModel):
    function: Literal[
        "models", "base_model", "supported", "credential", "environment",
        "url", "map", "upload", "response_sync", "response_async",
    ]
    model: str = ""
    field: Literal["api_key", "api_base"] = "api_key"
    value: str | None = None
    environment: dict[str, str] = {}
    api_key: str | None = None
    headers: dict[str, str] = {}
    api_base: str | None = None
    non_default_params: Obj = {}
    optional_params: Obj | None = None
    drop_params: bool = False
    file: FileSpec | None = None
    body: list[int] = []
    elapsed_ms: float = 0.0
    response_ms: float = 0.0
    created: int = 0


class Input(BaseModel):
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


def file_value(spec: FileSpec) -> FileTypes:
    if spec.kind == "bytes":
        return spec.content.encode()
    if spec.kind == "tuple2":
        return (spec.filename, spec.content.encode())
    if spec.kind == "tuple3":
        return (spec.filename, spec.content.encode(), spec.content_type)
    if spec.kind == "tuple4":
        return (spec.filename, spec.content.encode(), spec.content_type, spec.headers)
    return Path("image.png")


def prepared_json(prepared: Prepared) -> JsonValue:
    filename, content, content_type, headers = prepared
    encoded = None if content is None else base64.b64encode(content).decode()
    return {
        "filename": filename, "content": encoded,
        "content_type": content_type, "headers": headers,
    }


def response_json(response: ImageResponse) -> JsonValue:
    view = RESPONSE.validate_json(response.model_dump_json())
    return {
        "created": view["created"],
        "data": [
            {
                "b64_json": image.get("b64_json"), "url": image.get("url"),
                "revised_prompt": image.get("revised_prompt"),
            }
            for image in view["data"]
        ],
        "usage": view["usage"],
    }


def catalogue_answer(call: Call) -> JsonValue:
    if call.function == "models":
        return [str(name) for name in INFO.get_models()]
    if call.function == "base_model":
        return INFO.get_base_model(call.model)
    if call.function == "supported":
        return [str(name) for name in CONFIG.get_supported_openai_params(MODEL)]
    raise ValueError(f"{call.function} is not a catalogue call")


def credential_answer(call: Call) -> JsonValue:
    if call.function == "credential":
        os.environ.pop("TOPAZ_API_KEY", None)
        os.environ.pop("TOPAZ_API_BASE", None)
        os.environ.update(call.environment)
        if call.field == "api_key":
            return {"api_key": INFO.get_api_key(call.value)}
        return {"api_base": INFO.get_api_base(call.value)}
    if call.function == "environment":
        try:
            headers = CONFIG.validate_environment(
                call.headers, MODEL, NoMessages(), EMPTY, EMPTY, call.api_key, None)
        except ValueError:
            return {"missing_key": True}
        return {"headers": headers}
    raise ValueError(f"{call.function} is not a credential call")


def mapping_answer(call: Call) -> JsonValue:
    if call.function == "url":
        return {"url": CONFIG.get_complete_url(call.api_base, None, MODEL, EMPTY, EMPTY)}
    if call.function == "map":
        params = call.optional_params
        if params is None:
            raise ValueError("map call without optional_params")
        try:
            mapped = CONFIG.map_openai_params(
                call.non_default_params, params, MODEL, drop_params=call.drop_params)
        except AssertionError:
            return {"invalid": True}
        return {"mapped": mapped}
    raise ValueError(f"{call.function} is not a mapping call")


def upload_answer(call: Call) -> JsonValue:
    file = call.file
    if file is None:
        raise ValueError("upload call without a file")
    prepared = file_value(file)
    params = call.optional_params
    if params is None:
        return prepared_json(CONFIG.prepare_file_tuple(prepared))
    fields = CONFIG.transform_request_image_variation(MODEL, prepared, params, EMPTY)
    return {"files": {"image": prepared_json(fields["files"]["image"])}, "data": fields["data"]}


async def measure_async(call: Call) -> JsonValue:
    response = BodyResponse.__new__(BodyResponse)
    response.body = bytes(call.body)
    log = scripted_log()
    log.measured = call.response_ms
    finished = await CONFIG.async_transform_response_image_variation(
        MODEL, response, ImageResponse(created=call.created),
        log, EMPTY, b"pixels", EMPTY, EMPTY, None,
    )
    return {
        "response": response_json(finished),
        "response_ms": log.measured,
        "timing_source": "measured" if log.measures else "elapsed",
    }


def reply_answer(call: Call) -> JsonValue:
    if call.function == "response_async":
        return asyncio.run(measure_async(call))
    if call.function == "response_sync":
        response = httpx.Response(200, content=bytes(call.body))
        response.elapsed = timedelta(milliseconds=call.elapsed_ms)
        log = scripted_log()
        finished = CONFIG.transform_response_image_variation(
            MODEL, response, ImageResponse(created=call.created),
            log, EMPTY, b"pixels", EMPTY, EMPTY, None,
        )
        return {
            "response": response_json(finished),
            "response_ms": response.elapsed.total_seconds() * 1000,
            "timing_source": "measured" if log.measures else "elapsed",
        }
    raise ValueError(f"{call.function} is not a reply call")


def call_value(call: Call) -> JsonValue:
    match call.function:
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


def run(input_value: JsonValue) -> JsonValue:
    calls = Input.model_validate(input_value).calls
    return [call_value(call) for call in calls]
