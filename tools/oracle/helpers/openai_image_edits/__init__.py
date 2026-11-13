import base64
import io
import os
import time
from collections.abc import Awaitable
from functools import partial
from typing import Final, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.image_edit import get_openai_image_edit_config
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import FileTypes, ImageResponse
from openai import BadRequestError
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from openai_image_edits.multipart import Call, call_value

litellm.suppress_debug_info = True
time.time = partial(float, 1_700_000_000)

JSON: Final = TypeAdapter(JsonValue)
TIMEOUT: Final = 30.0
OPENAI: Final = "openai"
FAMILIES: Final[dict[str, str]] = {
    "DallE2ImageEditConfig": "dall_e_2",
    "OpenAIImageEditConfig": "openai",
}


class FileSpec(TypedDict):
    content_base64: str
    name: str | None


class Controls(TypedDict, total=False):
    api_key: str
    api_base: str
    organization: str
    openai_key: str


class Reply(TypedDict):
    status: int
    body: str


class Probe(TypedDict):
    model: str
    prompt: str | None
    image: list[FileSpec | None] | None
    mask: list[FileSpec | None] | None
    options: dict[str, JsonValue]
    params: Controls
    globals: Controls
    environment: dict[str, str]
    reply: Reply


class Input(TypedDict, total=False):
    transform: list[Probe]
    reply: list[Probe]


class Outcome(TypedDict):
    family: str
    supported: list[str]
    call: Call | None
    error: JsonValue


Options = dict[str, JsonValue | list[FileTypes | None]]


class ParamsConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...


class EditHandler(Protocol):
    def image_edit_handler(
        self,
        *call: *tuple[
            str,
            list[FileTypes | None] | None,
            str | None,
            BaseImageEditConfig,
            Options,
            str,
            GenericLiteLLMParams,
            Logging,
            float,
            None,
            None,
            HTTPHandler,
        ],
    ) -> ImageResponse | Awaitable[ImageResponse]: ...


class NamedBuffer(io.BufferedReader):
    def __init__(self, raw: io.BytesIO, name: str) -> None:
        super().__init__(raw)
        self._name = name

    @property
    @override
    def name(self) -> str:
        return self._name


class Fixture:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(
            self.reply["status"],
            headers={"x-request-id": "req-1"},
            content=self.reply["body"].encode(),
            request=request,
        )


class Logger(Logging):
    def __init__(self) -> None:
        self.seen: dict[str, str | None] = {}

    @override
    def pre_call(
        self,
        input: str | None = None,
        api_key: str = "",
        model: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        self.seen = {"input": input, "api_key": api_key, "model": model}
        del additional_args


def apply_globals(values: Controls) -> None:
    litellm.api_key = values.get("api_key")
    litellm.openai_key = values.get("openai_key")
    litellm.api_base = values.get("api_base")


def file_value(spec: FileSpec | None) -> FileTypes | None:
    if spec is None:
        return None
    data = base64.b64decode(spec["content_base64"])
    name = spec["name"]
    if name is None:
        return data
    return NamedBuffer(io.BytesIO(data), name)


def files_value(specs: list[FileSpec | None] | None) -> list[FileTypes | None] | None:
    if specs is None:
        return None
    return [file_value(spec) for spec in specs]


def params_value(params: Controls) -> GenericLiteLLMParams:
    return GenericLiteLLMParams(
        api_key=params.get("api_key"),
        api_base=params.get("api_base"),
        organization=params.get("organization"),
    )


def request_value(
    probe: Probe,
    handler: EditHandler,
    config: BaseImageEditConfig,
    params: ParamsConfig,
) -> Outcome:
    supported = params.get_supported_openai_params(probe["model"])
    mask = files_value(probe["mask"])
    options: Options = {**probe["options"]}
    if mask is not None:
        options["mask"] = mask
    apply_globals(probe["globals"])
    fixture = Fixture(probe["reply"])
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(fixture)))
    os.environ.clear()
    os.environ.update(probe["environment"])
    try:
        handler.image_edit_handler(
            probe["model"],
            files_value(probe["image"]),
            probe["prompt"],
            config,
            options,
            OPENAI,
            params_value(probe["params"]),
            Logger(),
            TIMEOUT,
            None,
            None,
            client,
        )
    except BadRequestError as failure:
        return Outcome(
            family=FAMILIES[type(config).__name__],
            supported=supported,
            call=None,
            error={"kind": "bad_request", "status": failure.status_code},
        )
    return Outcome(
        family=FAMILIES[type(config).__name__],
        supported=supported,
        call=call_value(fixture.requests[0]),
        error=None,
    )


def transform_value(probe: Probe) -> JsonValue:
    config = get_openai_image_edit_config(probe["model"])
    found = request_value(probe, BaseLLMHTTPHandler(), config, config)
    call = found["call"]
    value: dict[str, JsonValue] = {
        "family": found["family"],
        "supported": JSON.validate_python(found["supported"]),
        "url": JSON.validate_python(None if call is None else call["url"]),
        "headers": JSON.validate_python(None if call is None else call["headers"]),
        "data": JSON.validate_python(None if call is None else call["data"]),
        "files": JSON.validate_python(None if call is None else call["parts"]),
        "error": found["error"],
    }
    return JSON.validate_python(value)


def reply_value(probe: Probe) -> JsonValue:
    config = get_openai_image_edit_config(probe["model"])
    reply = probe["reply"]
    response = httpx.Response(
        reply["status"],
        headers={"x-request-id": "req-1"},
        content=reply["body"].encode(),
        request=httpx.Request("POST", "https://example.test/v1/images/edits"),
    )
    transformed = config.transform_image_edit_response(
        model=probe["model"], raw_response=response, logging_obj=Logger()
    )
    return JSON.validate_json(transformed.model_dump_json())


def run(input_value: Input) -> JsonValue:
    transform = input_value.get("transform")
    if transform is not None:
        transforms: list[JsonValue] = [transform_value(probe) for probe in transform]
        return JSON.validate_python(transforms)
    reply = input_value.get("reply")
    if reply is None:
        raise ValueError("the input names neither transform nor reply")
    replies: list[JsonValue] = [reply_value(probe) for probe in reply]
    return JSON.validate_python(replies)
