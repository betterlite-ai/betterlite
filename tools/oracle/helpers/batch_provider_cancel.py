import asyncio
import json
import os
import threading
from collections.abc import Coroutine
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import ClassVar, Final, Literal, Protocol, TypedDict, cast

import litellm
from litellm.batches import main as batch_main
from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue, TypeAdapter

WIRE: Final = TypeAdapter(JsonValue)
SIGNING: Final = {"aws_access_key_id": "AKIDEXAMPLE", "aws_secret_access_key": "example"}

Provider = Literal["openai", "azure", "vertex_ai", "bedrock", "litellm_proxy", "xai"]


class CancelFn(Protocol):
    def __call__(
        self,
        batch_id: str,
        model: str | None,
        custom_llm_provider: Provider,
        extra_headers: dict[str, str] | None,
        extra_body: dict[str, str] | None,
        **params: JsonValue,
    ) -> Coroutine[JsonValue, JsonValue, LiteLLMBatch]: ...


class StatusFn(Protocol):
    def __call__(self, batch_id: str, aws_region_name: str, **params: str) -> LiteLLMBatch: ...


CANCEL, ASYNC_STATUS = cast(CancelFn, litellm.acancel_batch), cast(StatusFn, batch_main._handle_async_invoke_status)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L888 https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/main.py#L1087


class Settings(TypedDict):
    api_base: str | None
    api_version: str | None
    api_key: str | None
    openai_key: str | None
    azure_key: str | None
    organization: str | None
    vertex_project: str | None
    vertex_location: str | None


class Params(TypedDict):
    api_base: str | None
    api_key: str | None
    api_version: str | None
    organization: str | None
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | None
    azure_ad_token: str | None
    aws_region_name: str | None
    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    timeout: float | None
    request_timeout: float | None
    max_retries: int | None


class CancelCall(TypedDict):
    batch_id: str
    model: str | None
    provider: Provider
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    params: Params
    call: dict[str, JsonValue]


class S3Output(TypedDict):
    s3Uri: str


class Status(TypedDict):
    invocationArn: str
    modelArn: str
    status: str
    failureMessage: str | None
    outputDataConfig: dict[str, JsonValue] | None
    submitTime: str | None
    lastModifiedTime: str | None
    endTime: str | None


class Envelope(TypedDict, total=False):
    cancel: CancelCall
    async_invoke: Status


class Script(TypedDict):
    batch: dict[str, JsonValue] | None
    error: str | None


class Input(TypedDict):
    input: Envelope
    settings: Settings | None
    env: dict[str, str]
    now: int
    script: Script


def cancel(call: CancelCall, settings: Settings | None, env: dict[str, str]) -> JsonValue:
    if "async" not in call["call"]:
        raise ValueError("the batch provider cancel helper drives the async entry point")
    os.environ.update(env)
    previous = (
        litellm.api_base,
        litellm.api_version,
        litellm.api_key,
        litellm.openai_key,
        litellm.azure_key,
        litellm.organization,
        litellm.vertex_project,
        litellm.vertex_location,
    )
    if settings is not None:
        litellm.api_base = settings["api_base"]
        litellm.api_version = settings["api_version"]
        litellm.api_key = settings["api_key"]
        litellm.openai_key = settings["openai_key"]
        litellm.azure_key = settings["azure_key"]
        litellm.vertex_project = settings["vertex_project"]
        litellm.vertex_location = settings["vertex_location"]
    try:
        batch = asyncio.run(
            CANCEL(
                batch_id=call["batch_id"],
                model=call["model"],
                custom_llm_provider=call["provider"],
                extra_headers=call["extra_headers"],
                extra_body=call["extra_body"],
                api_base=call["params"]["api_base"],
                api_key=call["params"]["api_key"],
                api_version=call["params"]["api_version"],
                organization=call["params"]["organization"],
                vertex_project=call["params"]["vertex_project"],
                vertex_location=call["params"]["vertex_location"],
                vertex_credentials=call["params"]["vertex_credentials"],
                azure_ad_token=call["params"]["azure_ad_token"],
                aws_region_name=call["params"]["aws_region_name"],
                aws_access_key_id=call["params"]["aws_access_key_id"],
                aws_secret_access_key=call["params"]["aws_secret_access_key"],
                aws_session_token=call["params"]["aws_session_token"],
                timeout=call["params"]["timeout"],
                request_timeout=call["params"]["request_timeout"],
                max_retries=call["params"]["max_retries"],
            )
        )
    finally:
        (
            litellm.api_base,
            litellm.api_version,
            litellm.api_key,
            litellm.openai_key,
            litellm.azure_key,
            litellm.organization,
            litellm.vertex_project,
            litellm.vertex_location,
        ) = previous
    answer: dict[str, JsonValue] = {
        "batch": WIRE.validate_json(batch.model_dump_json()),
        "model": call["model"],
        "calls": [],
    }
    return answer


def status_fields(status: Status) -> dict[str, JsonValue]:
    fields: dict[str, JsonValue] = {}
    fields["invocationArn"] = status["invocationArn"]
    fields["modelArn"] = status["modelArn"]
    fields["status"] = status["status"]
    fields["failureMessage"] = status["failureMessage"]
    fields["outputDataConfig"] = status["outputDataConfig"]
    fields["submitTime"] = status["submitTime"]
    fields["lastModifiedTime"] = status["lastModifiedTime"]
    fields["endTime"] = status["endTime"]
    return fields


class StatusServer(BaseHTTPRequestHandler):
    body: ClassVar[bytes] = b""

    def do_GET(self) -> None:
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(self.body)))
        self.end_headers()
        self.wfile.write(self.body)


def async_invoke(status: Status) -> JsonValue:
    StatusServer.body = json.dumps(status_fields(status)).encode()
    server = HTTPServer(("127.0.0.1", 0), StatusServer)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        batch = ASYNC_STATUS(
            batch_id=status["invocationArn"],
            aws_region_name="us-east-1",
            aws_bedrock_runtime_endpoint=f"http://127.0.0.1:{server.server_port}",
            **SIGNING,
        )
    finally:
        server.shutdown()
        thread.join()
    answer: dict[str, JsonValue] = {
        "batch": WIRE.validate_json(batch.model_dump_json()),
        "model": None,
        "calls": [],
    }
    return answer


def run(input_value: Input) -> JsonValue:
    envelope = input_value["input"]
    if "cancel" in envelope:
        return cancel(
            envelope["cancel"],
            input_value["settings"],
            input_value["env"],
        )
    if "async_invoke" in envelope:
        return async_invoke(envelope["async_invoke"])
    raise ValueError("the case names no batch provider cancel operation")
