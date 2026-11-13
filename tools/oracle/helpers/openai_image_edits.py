import base64
import io
import os
import time
from collections.abc import Generator
from contextlib import contextmanager
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
from litellm.types.utils import FileTypes
from openai import BadRequestError
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True
time.time = partial(float, 1_700_000_000)

JSON: Final = TypeAdapter(JsonValue)
NOW: Final = 1_700_000_000
TIMEOUT: Final = 30.0
OPENAI: Final = "openai"
HEADER_NAMES: Final = ("authorization", "openai-organization")
FAMILIES: Final[dict[str, str]] = {
    "DallE2ImageEditConfig": "dall_e_2",
    "OpenAIImageEditConfig": "openai",
}


class FileSpec(TypedDict):
    content_base64: str
    name: str | None


class Params(TypedDict, total=False):
    api_key: str
    api_base: str
    organization: str


class Globals(TypedDict, total=False):
    api_key: str
    openai_key: str
    api_base: str


class Reply(TypedDict):
    status: int
    body: str


class Probe(TypedDict):
    model: str
    prompt: str | None
    image: list[FileSpec | None] | None
    mask: list[FileSpec | None] | None
    options: dict[str, JsonValue]
    params: Params
    globals: Globals
    environment: dict[str, str]
    reply: Reply


class Input(TypedDict, total=False):
    transform: list[Probe]
    environment: list[Probe]
    reply: list[Probe]


class Part(TypedDict):
    name: str
    filename: str | None
    content_type: str | None
    content_base64: str


class Call(TypedDict):
    url: str
    headers: dict[str, str]
    data: dict[str, JsonValue]
    parts: list[Part]


class Outcome(TypedDict):
    family: str
    supported: list[str]
    call: Call | None
    error: JsonValue


class ParamsConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...


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
        self.seen: dict[str, object] = {}

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


@contextmanager
def environment(values: dict[str, str]) -> Generator[None, None, None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


def apply_globals(values: Globals) -> None:
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


def params_value(params: Params) -> GenericLiteLLMParams:
    return GenericLiteLLMParams(
        api_key=params.get("api_key"),
        api_base=params.get("api_base"),
        organization=params.get("organization"),
    )


def field_of(headers: str, marker: str) -> str | None:
    at = headers.find(marker)
    if at < 0:
        return None
    rest = headers[at + len(marker) :]
    end = rest.find('"')
    if end < 0:
        return rest
    return rest[:end]


def parts_of(request: httpx.Request) -> list[Part]:
    content_type = request.headers["content-type"]
    marker = "boundary="
    at = content_type.find(marker)
    if at < 0:
        raise ValueError("the image edit request is not multipart")
    delimiter = b"--" + content_type[at + len(marker) :].encode()
    parts: list[Part] = []
    for chunk in request.read().split(delimiter)[1:]:
        if chunk.startswith(b"--"):
            break
        head, _, data = chunk.partition(b"\r\n\r\n")
        fields = head.decode()
        name = field_of(fields, 'name="')
        if name is None:
            raise ValueError("a multipart part has no name")
        parts.append(
            Part(
                name=name,
                filename=field_of(fields, 'filename="'),
                content_type=field_of(fields, "Content-Type: "),
                content_base64=base64.b64encode(data.removesuffix(b"\r\n")).decode(),
            )
        )
    return parts


def call_value(fixture: Fixture) -> Call:
    request = fixture.requests[0]
    data: dict[str, JsonValue] = {}
    files: list[Part] = []
    for part in parts_of(request):
        if part["filename"] is None:
            data[part["name"]] = base64.b64decode(part["content_base64"]).decode()
        else:
            files.append(part)
    headers: dict[str, str] = {}
    for name in HEADER_NAMES:
        value = request.headers.get(name)
        if value is not None:
            headers[name] = value
    return Call(url=str(request.url), headers=headers, data=data, parts=files)


def supported_value(config: BaseImageEditConfig, model: str) -> list[str]:
    params: ParamsConfig = config
    return params.get_supported_openai_params(model)


def request_value(probe: Probe) -> Outcome:
    config = get_openai_image_edit_config(probe["model"])
    supported = supported_value(config, probe["model"])
    mask = files_value(probe["mask"])
    options: dict[str, object] = dict(probe["options"])
    if mask is not None:
        options["mask"] = mask
    apply_globals(probe["globals"])
    fixture = Fixture(probe["reply"])
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(fixture)))
    try:
        with environment(probe["environment"]):
            BaseLLMHTTPHandler().image_edit_handler(
                model=probe["model"],
                image=files_value(probe["image"]),
                prompt=probe["prompt"],
                image_edit_provider_config=config,
                image_edit_optional_request_params=options,
                custom_llm_provider=OPENAI,
                litellm_params=params_value(probe["params"]),
                logging_obj=Logger(),
                timeout=TIMEOUT,
                client=client,
            )
    except BadRequestError as failure:
        return Outcome(
            family=FAMILIES[type(config).__name__],
            supported=supported,
            call=None,
            error=error_value("bad_request", failure.status_code),
        )
    return Outcome(
        family=FAMILIES[type(config).__name__],
        supported=supported,
        call=call_value(fixture),
        error=None,
    )


def transform_value(probe: Probe) -> JsonValue:
    found = request_value(probe)
    call = found["call"]
    value: dict[str, JsonValue] = {
        "family": found["family"],
        "supported": JSON.validate_python(found["supported"]),
        "data": JSON.validate_python(None if call is None else call["data"]),
        "files": JSON.validate_python(None if call is None else call["parts"]),
        "error": found["error"],
    }
    return JSON.validate_python(value)


def environment_value(probe: Probe) -> JsonValue:
    found = request_value(probe)
    call = found["call"]
    value: dict[str, JsonValue] = {
        "url": JSON.validate_python(None if call is None else call["url"]),
        "headers": JSON.validate_python(None if call is None else call["headers"]),
        "error": found["error"],
    }
    return JSON.validate_python(value)
def error_value(kind: str, status: int | None) -> JsonValue:
    value: dict[str, JsonValue] = {"kind": kind, "status": status}
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
    if "transform" in input_value:
        transforms: list[JsonValue] = [
            transform_value(probe) for probe in input_value["transform"]
        ]
        return JSON.validate_python(transforms)
    if "environment" in input_value:
        environments: list[JsonValue] = [
            environment_value(probe) for probe in input_value["environment"]
        ]
        return JSON.validate_python(environments)
    replies: list[JsonValue] = [reply_value(probe) for probe in input_value["reply"]]
    return JSON.validate_python(replies)
