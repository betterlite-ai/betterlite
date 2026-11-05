import asyncio
import inspect
import json
import re
from collections.abc import AsyncGenerator, AsyncIterator, Iterator
from types import TracebackType
from typing import TYPE_CHECKING, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
import openai
from litellm.llms.openai.openai import OpenAIFilesAPI
from openai._response import AsyncResponseContextManager  # pyright: ignore[reportPrivateUsage]

if TYPE_CHECKING:
    from litellm.types.llms.openai import (
        CreateFileRequest,
        FileContentRequest,
        HttpxBinaryResponseContent,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Client: TypeAlias = openai.OpenAI | openai.AsyncOpenAI

DEFAULT_BASE = "https://api.openai.com/v1"
SDK_TIMEOUT = 600
CHUNK_SIZE = 1024 * 1024


class ReplyBody(httpx.AsyncByteStream, httpx.SyncByteStream):
    def __init__(self, chunks: list[bytes], *, fail_after: bool) -> None:
        self.chunks = chunks
        self.fail_after = fail_after

    def __iter__(self) -> Iterator[bytes]:
        yield from self.chunks
        if self.fail_after:
            raise httpx.ReadError("the mock connection broke")

    async def __aiter__(self) -> AsyncIterator[bytes]:
        for chunk in self.chunks:
            yield chunk
        if self.fail_after:
            raise httpx.ReadError("the mock connection broke")


class Upstream:
    def __init__(self, reply: Fields | None) -> None:
        self.reply = reply
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        if self.reply is None:
            raise httpx.ConnectError("the call needs no upstream", request=request)
        headers = [
            (str(pair[0]), str(pair[1]))
            for pair in cast("list[list[Json]]", self.reply.get("headers") or [])
        ]
        status = cast("int", self.reply["status"])
        if self.reply.get("json") is not None:
            return httpx.Response(
                status, headers=headers, content=json.dumps(self.reply["json"]).encode()
            )
        chunks = [
            bytes(cast("list[int]", chunk))
            for chunk in cast("list[Json]", self.reply.get("chunks") or [])
        ]
        body = ReplyBody(chunks, fail_after=bool(self.reply.get("fail_after")))
        return httpx.Response(status, headers=headers, stream=body)


class ExitLog:
    def __init__(self) -> None:
        self.kinds: list[str] = []

    async def record(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        del exc, exc_tb
        if exc_type is None:
            self.kinds.append("completed")
        elif exc_type is GeneratorExit:
            self.kinds.append("closed")
        else:
            self.kinds.append("failed")

    def kind(self) -> str:
        return self.kinds[0] if self.kinds else "unseen"


def part_view(raw: bytes) -> Json:
    head, _, body = raw.partition(b"\r\n\r\n")
    text = head.decode()
    name = re.search(r'name="([^"]*)"', text)
    filename = re.search(r'filename="([^"]*)"', text)
    content_type = re.search(r"(?im)^content-type: (.*)$", text)
    return {
        "name": name.group(1) if name else None,
        "filename": filename.group(1) if filename else None,
        "content_type": content_type.group(1).strip() if content_type else None,
        "text": body.removesuffix(b"\r\n").decode(),
    }


def multipart_view(request: httpx.Request) -> Json:
    content_type = cast("str", request.headers.get("content-type", ""))
    if not content_type.startswith("multipart/form-data"):
        return None
    boundary = content_type.split("boundary=", 1)[1].encode()
    pieces = [piece.removeprefix(b"\r\n") for piece in request.content.split(b"--" + boundary)]
    return [part_view(piece) for piece in pieces if piece.startswith(b"Content-Disposition")]


def request_view(upstream: Upstream, extra_headers: Fields) -> Json:
    if not upstream.seen:
        return None
    request = upstream.seen[0]
    path, _, _ = request.url.raw_path.decode().partition("?")
    timeout = cast("dict[str, float]", request.extensions["timeout"])
    return {
        "method": request.method,
        "path": path,
        "query": [[key, value] for key, value in request.url.params.multi_items()],
        "accept": request.headers.get("accept"),
        "authorization": request.headers.get("authorization"),
        "organization": request.headers.get("openai-organization"),
        "extra_headers": {name: request.headers.get(name) for name in extra_headers},
        "timeout": int(timeout["read"]),
        "parts": multipart_view(request),
    }


def build_client(spec: Fields, transport: httpx.MockTransport, *, is_async: bool) -> Client:
    timeout = spec.get("timeout")
    max_retries = spec.get("max_retries")
    if is_async:
        return openai.AsyncOpenAI(
            api_key=cast("str | None", spec.get("api_key")),
            base_url=cast("str", spec.get("api_base") or DEFAULT_BASE),
            organization=cast("str | None", spec.get("organization")),
            max_retries=cast("int", 0 if max_retries is None else max_retries),
            timeout=openai.not_given if timeout is None else cast("float", timeout),
            http_client=httpx.AsyncClient(transport=transport),
        )
    return openai.OpenAI(
        api_key=cast("str | None", spec.get("api_key")),
        base_url=cast("str", spec.get("api_base") or DEFAULT_BASE),
        organization=cast("str | None", spec.get("organization")),
        max_retries=cast("int", 0 if max_retries is None else max_retries),
        timeout=openai.not_given if timeout is None else cast("float", timeout),
        http_client=httpx.Client(transport=transport),
    )


def error_view(error: Exception) -> Json:
    if isinstance(error, openai.APIStatusError):
        return {
            "kind": "status",
            "status": error.status_code,
            "type": error.type,
            "param": error.param,
            "code": error.code,
            "message": error.message,
        }
    return {"kind": "ValueError", "message": str(error)}


def watched(reply: Fields, headers: httpx.Headers | dict[str, str]) -> Json:
    names = [str(pair[0]).lower() for pair in cast("list[list[Json]]", reply.get("headers") or [])]
    return {name: headers.get(name) for name in names}


def with_options(built: dict[str, object], request: Fields) -> dict[str, object]:
    for key in ("extra_headers", "extra_body", "timeout"):
        if request.get(key):
            built[key] = request[key]
    return built


def create_request(request: Fields) -> dict[str, object]:
    data = str(request.get("data") or "").encode()
    filename = cast("str | None", request.get("filename"))
    content_type = cast("str | None", request.get("content_type"))
    upload: object = data
    if filename is not None and content_type is not None:
        upload = (filename, data, content_type)
    elif filename is not None:
        upload = (filename, data)
    built: dict[str, object] = {"file": upload, "purpose": request.get("purpose") or "batch"}
    if request.get("expires_after") is not None:
        built["expires_after"] = {"anchor": "created_at", "seconds": request["expires_after"]}
    return with_options(built, request)


def content_request(request: Fields) -> dict[str, object]:
    return with_options({"file_id": request.get("file_id") or ""}, request)


class Common(TypedDict):
    _is_async: bool
    api_base: str
    api_key: None
    timeout: int
    max_retries: None
    organization: None
    client: Client | None


def invoke(action: str, client: Client | None, request: Fields, *, is_async: bool) -> object:
    api = OpenAIFilesAPI()
    common = Common(
        _is_async=is_async,
        api_base=DEFAULT_BASE,
        api_key=None,
        timeout=SDK_TIMEOUT,
        max_retries=None,
        organization=None,
        client=client,
    )
    file_id = str(request.get("file_id") or "")
    match action:
        case "create_file":
            data = cast("CreateFileRequest", create_request(request))
            return api.create_file(create_file_data=data, **common)
        case "file_content":
            data = cast("FileContentRequest", content_request(request))
            return api.file_content(file_content_request=data, **common)
        case "file_content_streaming":
            data = cast("FileContentRequest", content_request(request))
            size = cast("int", request.get("chunk_size") or CHUNK_SIZE)
            return api.file_content_streaming(file_content_request=data, chunk_size=size, **common)
        case "retrieve_file":
            return api.retrieve_file(file_id=file_id, **common)
        case "delete_file":
            return api.delete_file(file_id=file_id, **common)
        case _:
            purpose = cast("str | None", request.get("list_purpose"))
            return api.list_files(purpose=purpose, **common)


async def read_chunks(
    stream: AsyncGenerator[bytes, None], limit: int | None
) -> tuple[list[Json], bool]:
    chunks: list[Json] = []
    broke = False
    while limit is None or len(chunks) < limit:
        try:
            chunk = await anext(stream)
        except StopAsyncIteration:
            break
        except httpx.HTTPError:
            broke = True
            break
        chunks.append(list(chunk))
    await stream.aclose()
    return chunks, broke


async def render(
    action: str, outcome: object, reply: Fields, request: Fields, exits: ExitLog
) -> Json:
    if action == "file_content_streaming":
        pair = cast("tuple[AsyncGenerator[bytes, None], dict[str, str]]", outcome)
        chunks, broke = await read_chunks(pair[0], cast("int | None", request.get("read")))
        return {
            "headers": watched(reply, pair[1]),
            "chunks": chunks,
            "broke": broke,
            "exit": exits.kind(),
        }
    if action == "file_content":
        response = cast("HttpxBinaryResponseContent", outcome).response
        return {
            "status": response.status_code,
            "headers": watched(reply, response.headers),
            "body": list(response.content),
        }
    model = cast("openai.BaseModel", outcome)
    if action == "create_file":
        return cast("Json", model.model_dump(exclude_none=True))
    return cast("Json", model.model_dump(exclude_unset=True))


async def drive(fields: Fields, client: Client | None, upstream: Upstream, exits: ExitLog) -> Json:
    action = cast("str", fields["action"])
    request = cast("Fields", fields.get("request") or {})
    spec = cast("Fields", fields["client"])
    try:
        outcome = invoke(action, client, request, is_async=bool(spec["is_async"]))
        if inspect.isawaitable(outcome):
            outcome = cast("object", await outcome)
        return await render(action, outcome, upstream.reply or {}, request, exits)
    except (openai.APIStatusError, ValueError) as error:
        return {"raised": error_view(error)}


def lifecycle(fields: Fields) -> Json:
    spec = cast("Fields", fields["client"])
    upstream = Upstream(cast("Fields | None", fields.get("reply")))
    exits = ExitLog()
    supplied = spec.get("supplied")
    is_async = bool(spec["is_async"]) if supplied is None else supplied == "async"
    transport = httpx.MockTransport(upstream.handle)
    client = None if spec.get("missing") else build_client(spec, transport, is_async=is_async)
    with (
        patch.object(AsyncResponseContextManager, "__aexit__", new=exits.record),
        patch.object(OpenAIFilesAPI, "get_openai_client", return_value=None)
        if spec.get("missing")
        else patch.dict("os.environ"),
    ):
        result = asyncio.run(drive(fields, client, upstream, exits))
    extra = cast("Fields", cast("Fields", fields.get("request") or {}).get("extra_headers") or {})
    return {"request": request_view(upstream, extra), "result": result}


def client_view(fields: Fields) -> Json:
    spec = cast("Fields", fields["client"])
    api = OpenAIFilesAPI()
    supplied = spec.get("supplied")
    held: Client | None = None
    if supplied is not None:
        held = api.get_openai_client(
            api_key="sk-supplied",
            api_base="https://supplied.test/v1",
            timeout=cast("float", None),
            max_retries=None,
            organization=None,
            _is_async=supplied == "async",
        )
    built = api.get_openai_client(
        api_key=cast("str | None", spec.get("api_key")),
        api_base=cast("str | None", spec.get("api_base")),
        timeout=cast("float", spec.get("timeout")),
        max_retries=cast("int | None", spec.get("max_retries")),
        organization=cast("str | None", spec.get("organization")),
        client=held,
        _is_async=bool(spec["is_async"]),
    )
    assert built is not None  # noqa: S101
    timeout = built.timeout
    return {
        "kind": "async" if isinstance(built, openai.AsyncOpenAI) else "sync",
        "base_url": str(built.base_url),
        "timeout": int(
            timeout.read
            if isinstance(timeout, httpx.Timeout) and timeout.read is not None
            else cast("float", timeout)
        ),
        "max_retries": built.max_retries,
        "organization": built.organization,
        "key_is_given": built.api_key == spec.get("api_key"),
    }


def run(fields: Fields) -> Json:
    if fields["action"] == "client":
        return client_view(fields)
    return lifecycle(fields)
