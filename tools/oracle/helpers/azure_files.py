import asyncio
from collections.abc import Coroutine
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
import openai
import pydantic
from litellm.llms.azure.common_utils import BaseAzureLLM
from litellm.llms.azure.files.handler import AzureOpenAIFilesAPI
from litellm.types.llms.openai import CREATE_FILE_REQUESTS_PURPOSE
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
RAISED: Final[JsonObject] = {
    "class": "", "status": None, "type": None, "param": None, "code": None, "message": None,
}


class Expiry(TypedDict):
    anchor: Literal["created_at"]
    seconds: int


class Upload(TypedDict):
    file: bytes
    purpose: CREATE_FILE_REQUESTS_PURPOSE
    expires_after: NotRequired[Expiry]


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


def part_view(head: str, data: str) -> JsonValue:
    name = head.split('name="', 1)[1].split('"', 1)[0]
    filename = head.split('; filename="', 1)[1].split('"', 1)[0] if '; filename="' in head else ""
    content_type = ""
    if "\r\nContent-Type: " in head:
        content_type = head.split("\r\nContent-Type: ", 1)[1].split("\r\n", 1)[0]
    return {"name": name, "filename": filename, "content_type": content_type, "data": data}


def parts_of(content: str, content_type: str) -> JsonValue:
    boundary = content_type.split("boundary=", 1)[1].split(";", 1)[0].strip('"')
    pieces = [chunk.partition("\r\n\r\n") for chunk in content.split(f"--{boundary}")]
    return [
        part_view(head, data.removesuffix("\r\n"))
        for head, separator, data in pieces
        if separator and 'name="' in head
    ]


def request_view(request: httpx.Request) -> JsonValue:
    chosen: JsonObject = {
        name: value
        for name, value in request.headers.multi_items()
        if name in COMPARED or (name.startswith("x-") and not name.startswith("x-stainless-"))
    }
    seconds = request.headers.get_list("x-stainless-read-timeout")
    content_type = request.headers.get_list("content-type")
    milliseconds = None if not seconds else int(float(seconds[0]) * 1000)
    parts: JsonValue = None
    if content_type and content_type[0].startswith("multipart/form-data"):
        parts = parts_of(request.content.decode(), content_type[0])
    return {
        "method": request.method, "host": request.url.host,
        "path": request.url.raw_path.split(b"?")[0].decode(),
        "query": [[name, value] for name, value in request.url.params.multi_items()],
        "headers": chosen, "timeout_ms": milliseconds,
        "parts": parts,
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
            reply["status"], content=reply["body"].encode(), headers=reply["headers"]
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


def settle(result: ModelResult) -> pydantic.BaseModel:
    if asyncio.iscoroutine(result):
        return asyncio.run(result)
    return result


def make_client(kind: Supply) -> SdkClient:
    sync = httpx.Client(transport=httpx.MockTransport(STATE.respond))
    stream = httpx.AsyncClient(transport=httpx.MockTransport(STATE.respond))
    if kind == "open_ai":
        return OpenAI(api_key=KEY, base_url=f"{HOST}/v1/", http_client=sync)
    if kind == "async_open_ai":
        return AsyncOpenAI(api_key=KEY, base_url=f"{HOST}/v1/", http_client=stream)
    if kind == "azure_open_ai":
        return AzureOpenAI(api_key=KEY, api_version=VERSION, azure_endpoint=HOST, http_client=sync)
    return AsyncAzureOpenAI(
        api_key=KEY, api_version=VERSION, azure_endpoint=HOST, http_client=stream
    )


class ClientCache(Protocol):
    def flush_cache(self) -> None: ...


def session() -> None:
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    cache.flush_cache()
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(STATE.respond))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(STATE.respond))


def client_step(step: Step, client: SdkClient | None, *, is_async: bool) -> JsonValue:
    args = step["args"]
    built = API.get_azure_openai_client(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        api_key=args["api_key"], api_base=args["api_base"], api_version=args["api_version"],
        client=client, litellm_params=args.get("litellm_params"), _is_async=is_async,
    )
    if built is None:
        raise ValueError(CLIENT_MISSING)
    read: float | None = None
    match built.timeout:
        case httpx.Timeout():
            read = built.timeout.read
        case float():
            read = built.timeout
    return {
        "kind": type(built).__name__, "base_url": str(built.base_url),
        "query": JSON.validate_python(built.default_query), "max_retries": built.max_retries,
        "timeout_ms": None if read is None else int(read * 1000),
    }


def file_step(step: Step, client: SdkClient | None, *, is_async: bool) -> JsonValue:
    args = step["args"]
    params = cast("dict[str, object] | None", args.get("litellm_params"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L53-L87
    action = step["action"]
    if action == "create":
        content = step.get("content")
        purpose = step.get("create_purpose")
        if content is None or purpose is None:
            raise ValueError("a create step carries no content or create purpose")
        data: Upload = {"file": content.encode(), "purpose": PURPOSE.validate_python(purpose)}
        expires_after = step.get("expires_after")
        if expires_after is not None:
            data["expires_after"] = {"anchor": "created_at", "seconds": expires_after}
        created = API.create_file(
            _is_async=is_async, create_file_data=data, api_base=args["api_base"],
            api_key=args["api_key"], api_version=args["api_version"], client=client,
            timeout=args["timeout"], max_retries=args["max_retries"], litellm_params=params,
        )
        return JSON.validate_json(settle(created).model_dump_json())
    if action == "list":
        listed = settle(cast("ModelResult", API.list_files(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L245-L285
            _is_async=is_async, purpose=step.get("purpose"), api_base=args["api_base"],
            api_key=args["api_key"], api_version=args["api_version"], client=client,
            timeout=args["timeout"], max_retries=args["max_retries"], litellm_params=params,
        )))
        return JSON.validate_json(listed.model_dump_json())
    file_id = step.get("file_id")
    if file_id is None:
        raise ValueError(f"a {action} step carries no file_id")
    if action == "content":
        pending = API.file_content(
            _is_async=is_async, api_base=args["api_base"], api_version=args["api_version"],
            api_key=args["api_key"], file_content_request={"file_id": file_id}, client=client,
            timeout=args["timeout"], max_retries=args["max_retries"], litellm_params=params,
        )
        response = asyncio.run(pending) if asyncio.iscoroutine(pending) else pending
        return {
            "status": response.response.status_code, "headers": dict(response.response.headers),
            "body": response.content.decode(),
        }
    if action == "retrieve":
        retrieved = settle(cast("ModelResult", API.retrieve_file(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L143-L179
            _is_async=is_async, file_id=file_id, api_base=args["api_base"],
            api_key=args["api_key"], api_version=args["api_version"], client=client,
            timeout=args["timeout"], max_retries=args["max_retries"], litellm_params=params,
        )))
        return JSON.validate_json(retrieved.model_dump_json())
    deleted = settle(cast("ModelResult", API.delete_file(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L192-L232
        _is_async=is_async, file_id=file_id, api_base=args["api_base"],
        api_key=args["api_key"], api_version=args["api_version"], client=client,
        timeout=args["timeout"], max_retries=args["max_retries"], litellm_params=params,
    )))
    return JSON.validate_json(deleted.model_dump_json())


def run_step(step: Step) -> JsonValue:
    STATE.calls.clear()
    STATE.reply = step.get("reply")
    builder = step.get("builder")
    STATE.builder = "default" if builder is None else builder
    session()
    is_async = bool(step.get("is_async"))
    kind = step.get("client")
    client = None if kind is None else make_client(kind)
    try:
        if step["action"] == "client":
            result: JsonValue = client_step(step, client, is_async=is_async)
        else:
            result = file_step(step, client, is_async=is_async)
        error: JsonValue = None
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


def run(input_value: dict[str, list[Step]]) -> JsonValue:
    steps: list[JsonValue] = [run_step(step) for step in input_value["steps"]]
    return {"steps": steps}
