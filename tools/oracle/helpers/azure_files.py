import asyncio
import re
from collections.abc import Coroutine
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
import openai
import pydantic
from litellm.llms.azure.common_utils import BaseAzureLLM
from litellm.llms.azure.files.handler import AzureOpenAIFilesAPI
from litellm.types.llms.openai import (
    CREATE_FILE_REQUESTS_PURPOSE,
    CreateFileRequest,
    FileContentRequest,
    HttpxBinaryResponseContent,
)
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue, TypeAdapter

HOST = "https://supplied.test"
KEY = "supplied-key"
VERSION = "2025-01-01"
CLIENT_MISSING = (
    "AzureOpenAI client is not initialized. Make sure api_key is passed "
    "or OPENAI_API_KEY is set in the environment."
)

JSON = TypeAdapter(JsonValue)
PURPOSE = TypeAdapter(CREATE_FILE_REQUESTS_PURPOSE)
JsonObject: TypeAlias = dict[str, JsonValue]
SdkClient: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI
ModelResult: TypeAlias = pydantic.BaseModel | Coroutine[object, object, pydantic.BaseModel]
Supply: TypeAlias = Literal["azure_open_ai", "async_azure_open_ai", "open_ai", "async_open_ai"]
COMPARED: Final[tuple[str, ...]] = ("accept", "api-key", "authorization", "openai-organization")
PART = re.compile(
    r'Content-Disposition: form-data; name="([^"]+)"(?:; filename="([^"]*)")?'
    r'(?:\r\nContent-Type: ([^\r]+))?\r\n\r\n(.*?)\r\n(?=--)', re.S
)
RAISED: Final[JsonObject] = {
    "class": "", "status": None, "type": None, "param": None, "code": None, "message": None,
}


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Values(TypedDict):
    api_key: str | None
    api_base: str | None
    api_version: str | None
    timeout: float
    max_retries: int
    litellm_params: NotRequired[JsonObject | None]


class Step(TypedDict):
    action: Literal["client", "create", "content", "retrieve", "list", "delete"]
    args: Values
    is_async: NotRequired[bool]
    client: NotRequired[Supply]
    builder: NotRequired[Literal["default", "none"]]
    content: NotRequired[str]
    purpose: NotRequired[str]
    create_purpose: NotRequired[str]
    expires_after: NotRequired[int]
    file_id: NotRequired[str]
    reply: NotRequired[Reply]


class Input(TypedDict):
    steps: list[Step]


class ClientCache(Protocol):
    def flush_cache(self) -> None: ...


def request_view(request: httpx.Request) -> JsonValue:
    chosen: JsonObject = {}
    for name, value in request.headers.multi_items():
        compared = name in COMPARED or (
            name.startswith("x-") and not name.startswith("x-stainless-")
        )
        if compared:
            chosen[name] = value
    seconds = request.headers.get("x-stainless-read-timeout")
    content_type = request.headers.get("content-type")
    multipart = content_type is not None and content_type.startswith("multipart/form-data")
    return {
        "method": request.method,
        "host": request.url.host,
        "path": request.url.raw_path.split(b"?")[0].decode(),
        "query": [[name, value] for name, value in request.url.params.multi_items()],
        "headers": chosen,
        "timeout_ms": None if seconds is None else int(float(seconds) * 1000),
        "parts": [
            {"name": name, "filename": file_name, "content_type": kind, "data": data}
            for name, file_name, kind, data in PART.findall(request.content.decode())
        ]
        if multipart
        else None,
    }


class State:
    def __init__(self) -> None:
        self.builder: Literal["default", "none"] = "default"
        self.reply: Reply | None = None
        self.calls: list[JsonValue] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(request_view(request))
        reply = STATE.reply
        if reply is None:
            return httpx.Response(200, content=b"")
        return httpx.Response(
            reply["status"],
            content=reply["body"].encode(),
            headers=reply["headers"],
        )


STATE = State()
API = AzureOpenAIFilesAPI()

REAL_AZURE_CLIENT = BaseAzureLLM.get_azure_openai_client  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489


def build_client(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
    self: BaseAzureLLM,
    *,
    api_key: str | None,
    api_base: str | None,
    api_version: str | None = None,
    client: SdkClient | None = None,
    litellm_params: dict[str, JsonValue] | None = None,
    _is_async: bool = False,
    model: str | None = None,
) -> SdkClient | None:
    if STATE.builder == "none":
        return None
    return REAL_AZURE_CLIENT(
        self, api_key, api_base, api_version, client, litellm_params, _is_async, model
    )


BaseAzureLLM.get_azure_openai_client = build_client  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L53-L87


def settle(result: ModelResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case object():
            return JSON.validate_json(result.model_dump_json())


def make_client(kind: Supply) -> SdkClient:
    wire = httpx.MockTransport(STATE.respond)
    sync = httpx.Client(transport=wire)
    stream = httpx.AsyncClient(transport=wire)
    if kind == "open_ai":
        return OpenAI(api_key=KEY, base_url=f"{HOST}/v1/", http_client=sync)
    if kind == "async_open_ai":
        return AsyncOpenAI(api_key=KEY, base_url=f"{HOST}/v1/", http_client=stream)
    if kind == "azure_open_ai":
        return AzureOpenAI(api_key=KEY, api_version=VERSION, azure_endpoint=HOST, http_client=sync)
    return AsyncAzureOpenAI(
        api_key=KEY, api_version=VERSION, azure_endpoint=HOST, http_client=stream
    )


def session() -> None:
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    cache.flush_cache()
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(STATE.respond))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(STATE.respond))


def client_step(step: Step, client: SdkClient | None, *, is_async: bool) -> JsonValue:
    args = step["args"]
    built = API.get_azure_openai_client(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        api_key=args.get("api_key"),
        api_base=args.get("api_base"),
        api_version=args.get("api_version"),
        client=client,
        litellm_params=args.get("litellm_params"),
        _is_async=is_async,
    )
    if built is None:
        raise ValueError(CLIENT_MISSING)
    match built.timeout:
        case httpx.Timeout():
            read = built.timeout.read
        case object():
            read = built.timeout
    return {
        "kind": type(built).__name__,
        "base_url": str(built.base_url),
        "query": JSON.validate_python(built.default_query),
        "timeout_ms": None if read is None else int(read * 1000),
        "max_retries": built.max_retries,
    }


def create_step(step: Step, client: SdkClient | None, *, is_async: bool) -> JsonValue:
    request: CreateFileRequest = {
        "file": step["content"].encode(),
        "purpose": PURPOSE.validate_python(step["create_purpose"]),
    }
    expires_after = step.get("expires_after")
    if expires_after is not None:
        request["expires_after"] = {"anchor": "created_at", "seconds": expires_after}
    created = API.create_file(_is_async=is_async, create_file_data=request, client=client, **step["args"])  # type: ignore[arg-type, misc]  # pyright: ignore[reportArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L53-L87
    return settle(created)


def content_step(step: Step, client: SdkClient | None, *, is_async: bool) -> JsonValue:
    content: FileContentRequest = {"file_id": step["file_id"]}
    pending = API.file_content(_is_async=is_async, file_content_request=content, client=client, **step["args"])  # type: ignore[arg-type, misc]  # pyright: ignore[reportArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L97-L133
    match pending:
        case Coroutine():
            value = asyncio.run(pending)
        case object():
            value = pending
    return {
        "status": value.response.status_code,
        "headers": dict(value.response.headers),
        "body": value.content.decode(),
    }


def file_step(step: Step, client: SdkClient | None, *, is_async: bool) -> JsonValue:
    args = step["args"]
    if step["action"] == "retrieve":
        pending = cast(ModelResult, API.retrieve_file(_is_async=is_async, file_id=step["file_id"], client=client, **args))  # type: ignore[arg-type, misc]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L143-L179
        return settle(pending)
    if step["action"] == "list":
        pending = cast(ModelResult, API.list_files(_is_async=is_async, purpose=step.get("purpose"), client=client, **args))  # type: ignore[arg-type, misc]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L245-L285
        return settle(pending)
    pending = cast(ModelResult, API.delete_file(_is_async=is_async, file_id=step["file_id"], client=client, **args))  # type: ignore[arg-type, misc]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L192-L232
    return settle(pending)


def act(step: Step) -> JsonValue:
    is_async = bool(step.get("is_async"))
    kind = step.get("client")
    client = None if kind is None else make_client(kind)
    action = step["action"]
    if action == "client":
        return client_step(step, client, is_async=is_async)
    if action == "create":
        return create_step(step, client, is_async=is_async)
    if action == "content":
        return content_step(step, client, is_async=is_async)
    return file_step(step, client, is_async=is_async)


def run_step(step: Step) -> JsonValue:
    STATE.calls.clear()
    STATE.reply = step.get("reply")
    builder = step.get("builder")
    STATE.builder = "default" if builder is None else builder
    session()
    try:
        result: JsonValue = act(step)
        error: JsonValue = None
    except pydantic.ValidationError:
        result, error = None, {**RAISED, "class": "ValidationError"}
    except openai.APIStatusError as failure:
        result, error = None, {
            "class": "APIStatusError",
            "status": failure.status_code,
            "type": failure.type,
            "param": failure.param,
            "code": failure.code,
            "message": None,
        }
    except openai.OpenAIError as failure:
        result, error = None, {**RAISED, "class": "OpenAIError", "message": str(failure)}
    except ValueError as failure:
        result, error = None, {**RAISED, "class": "ValueError", "message": str(failure)}
    request = STATE.calls[0] if STATE.calls else None
    return {"request": request, "result": result, "error": error}


def run(input_value: Input) -> JsonValue:
    steps: list[JsonValue] = [run_step(step) for step in input_value["steps"]]
    return {"steps": steps}
