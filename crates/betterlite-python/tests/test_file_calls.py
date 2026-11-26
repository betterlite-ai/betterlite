import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable, Callable, Iterator, Mapping
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypeAlias, runtime_checkable
from urllib.parse import urlsplit

import openai
import pytest
from batch_server import parameter_rows
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

import betterlite.files as files_package
import betterlite.files.main as files_main
from betterlite._completion.errors import ERROR_PAYLOAD, UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue
from betterlite._native import NativeFileCallError
from betterlite.exceptions import NotFoundError
from betterlite.files import (
    acreate_file,
    afile_delete,
    afile_list,
    afile_retrieve,
    create_file,
    file_delete,
    file_list,
    file_retrieve,
)
from betterlite.files._calls import FileDeleted, FilePage
from betterlite.types.llms.openai.core.aliases import FileTypes
from betterlite.types.llms.openai.core.files import OpenAIFileObject

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
NOT_FOUND: Final = 404
GOOD_KEY: Final = "sk-test"
FILE_ID: Final = "file-abc123"
MISSING_ID: Final = "file-missing"
FILES_PATH: Final = "/v1/files"
CREATED_AT: Final = 1710000000
PAYLOAD: Final = b"row1\n"
CREATE_FILENAME: Final = "rows.jsonl"
TUPLE_FILENAME: Final = "x.jsonl"
UPLOAD_NAME: Final = "upload"
CRLF: Final = b"\r\n"
HEADER_END: Final = b"\r\n\r\n"
# the openai SDK's own names for the two upload shapes litellm forwards
SERVER_FILE: Final[dict[str, JsonValue]] = {
    "id": FILE_ID,
    "bytes": len(PAYLOAD),
    "created_at": CREATED_AT,
    "filename": CREATE_FILENAME,
    "object": "file",
    "purpose": "batch",
    "status": "processed",
}
FILE_RECORD: Final[dict[str, JsonValue]] = {**SERVER_FILE, "expires_at": None, "status_details": None}
SERVER_PAGE: Final[dict[str, JsonValue]] = {
    "object": "list",
    "data": [SERVER_FILE],
    "has_more": False,
    "first_id": FILE_ID,
    "last_id": FILE_ID,
}
PAGE_RECORD: Final[dict[str, JsonValue]] = {**SERVER_PAGE, "data": [FILE_RECORD]}
DELETED_RECORD: Final[dict[str, JsonValue]] = {"id": FILE_ID, "object": "file", "deleted": True}
ERROR_DETAILS: Final[dict[str, JsonValue]] = {
    "code": "file_not_found",
    "type": "invalid_request_error",
    "param": "file_id",
}
EXPECTED_EXPORTS: Final[tuple[str, ...]] = (
    "FileContentStreamingResponse",
    "acreate_file",
    "afile_content",
    "afile_delete",
    "afile_list",
    "afile_retrieve",
    "create_file",
    "file_content",
    "file_delete",
    "file_list",
    "file_retrieve",
)
JSON_RECORD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Part(TypedDict):
    filename: str | None
    content_type: str | None
    body: bytes


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class ReferenceCall(Protocol):
    def __call__(self, *args: CallValue, **kwargs: CallValue) -> object: ...


class LiteLLMFiles(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


@runtime_checkable
class Dumped(Protocol):
    def model_dump_json(self) -> str: ...


@runtime_checkable
class DumpedPage(Dumped, Protocol):
    def __iter__(self) -> Iterator[object]: ...


REFERENCE: LiteLLMFiles = importlib.import_module("litellm")  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L115-L804


def error_payload(message: str) -> dict[str, JsonValue]:
    return {
        "error": {
            "message": message,
            "type": "invalid_request_error",
            "code": "file_not_found",
            "param": "file_id",
        },
    }


def field_value(header: str, key: str) -> str | None:
    for segment in header.split(";"):
        name, _, value = segment.strip().partition("=")
        if name == key:
            return value.strip().strip('"')
    return None


# the multipart body each upload shape reaches the server with, by part name
def sent_parts(call: RecordedCall) -> dict[str, Part]:
    lowered = {name.lower(): value for name, value in call["headers"].items()}
    kind = lowered.get("content-type")
    if kind is None or "multipart/form-data" not in kind:
        return {}
    marker = "boundary="
    start = kind.find(marker)
    if start < 0:
        return {}
    boundary = kind[start + len(marker):].split(";")[0].strip('"').encode()
    parts: dict[str, Part] = {}
    for chunk in call["body"].split(b"--" + boundary):
        section = chunk.removeprefix(CRLF)
        if not section.startswith(b"Content-Disposition:"):
            continue
        head, separator, content = section.partition(HEADER_END)
        if not separator:
            continue
        lines = head.decode().split(CRLF.decode())
        name = field_value(lines[0], "name")
        if name is None:
            continue
        content_type: str | None = None
        for line in lines[1:]:
            field, _, value = line.partition(":")
            if field.lower() == "content-type":
                content_type = value.strip()
        parts[name] = {
            "filename": field_value(lines[0], "filename"),
            "content_type": content_type,
            "body": content.removesuffix(CRLF),
        }
    return parts


class FileCallServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1"

    def record(self, method: str, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append(
            {"method": method, "path": path, "headers": headers, "body": body}
        )

    def last(self) -> RecordedCall:
        return self.calls[-1]

    @override
    def __enter__(self) -> Self:
        return self

    @override
    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[FileCallServer]
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:
        self.handle_call()

    def do_GET(self) -> None:
        self.handle_call()

    def do_DELETE(self) -> None:
        self.handle_call()

    def handle_call(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.command, self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, error_payload("invalid api key"))
            return
        target = urlsplit(self.path).path
        if target == f"{FILES_PATH}/{MISSING_ID}":
            self.reply(NOT_FOUND, error_payload("no such file"))
        elif self.command == "POST" and target == FILES_PATH:
            self.reply(OK, SERVER_FILE)
        elif self.command == "GET" and target == FILES_PATH:
            self.reply(OK, SERVER_PAGE)
        elif self.command == "GET" and target == f"{FILES_PATH}/{FILE_ID}":
            self.reply(OK, SERVER_FILE)
        elif self.command == "DELETE" and target == f"{FILES_PATH}/{FILE_ID}":
            self.reply(OK, DELETED_RECORD)
        else:
            self.reply(NOT_FOUND, error_payload("no such endpoint"))

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *args: object) -> None:
        pass


def dumped(value: object) -> dict[str, JsonValue]:
    match value:
        case Dumped():
            return JSON_RECORD.validate_json(value.model_dump_json())
        case object():
            raise AssertionError("the call did not answer with a file record")


def dumped_items(value: object) -> list[dict[str, JsonValue]]:
    match value:
        case DumpedPage():
            return [dumped(item) for item in value]
        case object():
            raise AssertionError("the call did not answer with a file page")


async def awaited_file(
    value: OpenAIFileObject | Awaitable[OpenAIFileObject],
) -> OpenAIFileObject:
    match value:
        case Awaitable():
            return await value
        case object():
            raise AssertionError("the call answered with a file object instead of a coroutine")


async def awaited_deleted(value: FileDeleted | Awaitable[FileDeleted]) -> FileDeleted:
    match value:
        case Awaitable():
            return await value
        case object():
            raise AssertionError("the call answered with a deletion instead of a coroutine")


async def awaited_page(value: FilePage | Awaitable[FilePage]) -> FilePage:
    match value:
        case Awaitable():
            return await value
        case object():
            raise AssertionError("the call answered with a page instead of a coroutine")


def native_cause(error: BaseException) -> NativeFileCallError:
    cause = error.__cause__
    match cause:
        case NativeFileCallError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


OurCall: TypeAlias = Callable[..., object]
CALL_PAIRS: Final[tuple[tuple[OurCall, OurCall], ...]] = (
    (create_file, REFERENCE.create_file),
    (acreate_file, REFERENCE.acreate_file),
    (file_retrieve, REFERENCE.file_retrieve),
    (afile_retrieve, REFERENCE.afile_retrieve),
    (file_delete, REFERENCE.file_delete),
    (afile_delete, REFERENCE.afile_delete),
    (file_list, REFERENCE.file_list),
    (afile_list, REFERENCE.afile_list),
)
ASYNC_CALLS: Final[tuple[OurCall, ...]] = (
    acreate_file,
    afile_retrieve,
    afile_delete,
    afile_list,
)
SYNC_CALLS: Final[tuple[OurCall, ...]] = (
    create_file,
    file_retrieve,
    file_delete,
    file_list,
)
EXPORTED: Final[tuple[tuple[object, object], ...]] = (
    (files_package.FileContentStreamingResponse, files_main.FileContentStreamingResponse),
    (files_package.acreate_file, files_main.acreate_file),
    (files_package.afile_content, files_main.afile_content),
    (files_package.afile_delete, files_main.afile_delete),
    (files_package.afile_list, files_main.afile_list),
    (files_package.afile_retrieve, files_main.afile_retrieve),
    (files_package.create_file, files_main.create_file),
    (files_package.file_content, files_main.file_content),
    (files_package.file_delete, files_main.file_delete),
    (files_package.file_list, files_main.file_list),
    (files_package.file_retrieve, files_main.file_retrieve),
)


def test_signature_matches_litellm() -> None:
    for ours, reference in CALL_PAIRS:
        assert parameter_rows(inspect.signature(ours)) == parameter_rows(
            inspect.signature(reference)
        )
    for call in ASYNC_CALLS:
        assert inspect.iscoroutinefunction(call)
    for call in SYNC_CALLS:
        assert not inspect.iscoroutinefunction(call)


def test_create_bytes_matches_litellm() -> None:
    with FileCallServer() as server:
        reference = REFERENCE.create_file(
            file=PAYLOAD,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = create_file(
            file=PAYLOAD,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    assert ours_call["method"] == reference_call["method"] == "POST"
    assert ours_call["path"] == reference_call["path"] == FILES_PATH
    reference_parts = sent_parts(reference_call)
    ours_parts = sent_parts(ours_call)
    assert ours_parts["file"]["filename"] == reference_parts["file"]["filename"] == UPLOAD_NAME
    assert ours_parts["file"]["body"] == reference_parts["file"]["body"] == PAYLOAD
    assert ours_parts["purpose"]["body"] == reference_parts["purpose"]["body"] == b"batch"
    assert dumped(ours) == dumped(reference) == FILE_RECORD


def test_create_path_matches_litellm(tmp_path: Path) -> None:
    rows = tmp_path / CREATE_FILENAME
    rows.write_bytes(PAYLOAD)
    with FileCallServer() as server:
        reference = REFERENCE.create_file(
            file=rows,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = create_file(
            file=rows,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    reference_parts = sent_parts(reference_call)
    ours_parts = sent_parts(ours_call)
    assert ours_parts["file"]["filename"] == reference_parts["file"]["filename"] == CREATE_FILENAME
    assert ours_parts["file"]["body"] == reference_parts["file"]["body"] == PAYLOAD
    assert dumped(ours) == dumped(reference) == FILE_RECORD


def test_create_file_object_matches_litellm(tmp_path: Path) -> None:
    rows = tmp_path / CREATE_FILENAME
    rows.write_bytes(PAYLOAD)
    with FileCallServer() as server, rows.open("rb") as stream:
        reference = REFERENCE.create_file(
            file=stream,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        stream.seek(0)
        position = stream.tell()
        ours = create_file(
            file=stream,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert stream.tell() == position
        ours_call = server.last()
    reference_parts = sent_parts(reference_call)
    ours_parts = sent_parts(ours_call)
    assert ours_parts["file"]["filename"] == reference_parts["file"]["filename"] == CREATE_FILENAME
    assert ours_parts["file"]["body"] == reference_parts["file"]["body"] == PAYLOAD
    assert dumped(ours) == dumped(reference) == FILE_RECORD


def test_create_tuple_matches_litellm() -> None:
    with FileCallServer() as server:
        reference = REFERENCE.create_file(
            file=(TUPLE_FILENAME, PAYLOAD),
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = create_file(
            file=(TUPLE_FILENAME, PAYLOAD),
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    reference_parts = sent_parts(reference_call)
    ours_parts = sent_parts(ours_call)
    assert ours_parts["file"]["filename"] == reference_parts["file"]["filename"] == TUPLE_FILENAME
    assert ours_parts["file"]["body"] == reference_parts["file"]["body"] == PAYLOAD
    assert dumped(ours) == dumped(reference) == FILE_RECORD


def test_create_expires_after_reaches_server() -> None:
    with FileCallServer() as server:
        created = create_file(
            file=PAYLOAD,
            purpose="batch",
            expires_after={"anchor": "created_at", "seconds": 3600},
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        parts = sent_parts(server.last())
    assert parts["expires_after[anchor]"]["body"] == b"created_at"
    assert parts["expires_after[seconds]"]["body"] == b"3600"
    assert dumped(created) == FILE_RECORD


def test_create_flag_coroutine() -> None:
    with FileCallServer() as server:
        flagged = create_file(
            file=PAYLOAD,
            purpose="batch",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            acreate_file=True,
        )
        assert inspect.isawaitable(flagged)
        made = asyncio.run(awaited_file(flagged))
        direct = asyncio.run(
            acreate_file(
                file=PAYLOAD,
                purpose="batch",
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert dumped(made) == dumped(direct) == FILE_RECORD


def test_retrieve_matches_litellm() -> None:
    with FileCallServer() as server:
        reference = REFERENCE.file_retrieve(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = file_retrieve(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    assert ours_call["method"] == reference_call["method"] == "GET"
    assert ours_call["path"] == reference_call["path"] == f"{FILES_PATH}/{FILE_ID}"
    assert dumped(ours) == dumped(reference) == FILE_RECORD


def test_retrieve_flag_coroutine() -> None:
    with FileCallServer() as server:
        flagged = file_retrieve(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            afile_retrieve=True,
        )
        assert inspect.isawaitable(flagged)
        asynchronous = file_retrieve(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            is_async=True,
        )
        assert inspect.isawaitable(asynchronous)
        made = asyncio.run(awaited_file(flagged))
        made_async = asyncio.run(awaited_file(asynchronous))
        direct = asyncio.run(
            afile_retrieve(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert dumped(made) == dumped(made_async) == dumped(direct) == FILE_RECORD


def test_delete_matches_litellm() -> None:
    with FileCallServer() as server:
        reference = REFERENCE.file_delete(
            file_id=FILE_ID,
            model="gpt-4o",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = file_delete(
            file_id=FILE_ID,
            model="gpt-4o",
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    assert ours_call["method"] == reference_call["method"] == "DELETE"
    assert ours_call["path"] == reference_call["path"] == f"{FILES_PATH}/{FILE_ID}"
    assert dumped(ours) == dumped(reference) == DELETED_RECORD


def test_delete_flag_coroutine() -> None:
    with FileCallServer() as server:
        flagged = file_delete(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            afile_delete=True,
        )
        assert inspect.isawaitable(flagged)
        asynchronous = file_delete(
            file_id=FILE_ID,
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
            is_async=True,
        )
        assert inspect.isawaitable(asynchronous)
        made = asyncio.run(awaited_deleted(flagged))
        made_async = asyncio.run(awaited_deleted(asynchronous))
        direct = asyncio.run(
            afile_delete(
                file_id=FILE_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
                model="gpt-4o",
            )
        )
    assert dumped(made) == dumped(made_async) == dumped(direct) == DELETED_RECORD


def test_list_matches_litellm() -> None:
    with FileCallServer() as server:
        reference = REFERENCE.file_list(
            custom_llm_provider="openai",
            purpose="batch",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = file_list(
            custom_llm_provider="openai",
            purpose="batch",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    assert ours_call["method"] == reference_call["method"] == "GET"
    assert ours_call["path"] == reference_call["path"] == f"{FILES_PATH}?purpose=batch"
    assert dumped(ours) == dumped(reference) == PAGE_RECORD
    assert dumped_items(ours) == dumped_items(reference) == [FILE_RECORD]
    assert dumped(ours)["has_more"] is False


def test_list_without_purpose_matches_litellm() -> None:
    with FileCallServer() as server:
        reference = REFERENCE.file_list(
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        ours = file_list(
            custom_llm_provider="openai",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        ours_call = server.last()
    assert ours_call["path"] == reference_call["path"] == FILES_PATH
    assert dumped(ours) == dumped(reference) == PAGE_RECORD


def test_list_flag_coroutine() -> None:
    with FileCallServer() as server:
        flagged = file_list(
            custom_llm_provider="openai",
            purpose="batch",
            api_base=server.url,
            api_key=GOOD_KEY,
            afile_list=True,
        )
        assert inspect.isawaitable(flagged)
        asynchronous = file_list(
            custom_llm_provider="openai",
            purpose="batch",
            api_base=server.url,
            api_key=GOOD_KEY,
            is_async=True,
        )
        assert inspect.isawaitable(asynchronous)
        made = asyncio.run(awaited_page(flagged))
        made_async = asyncio.run(awaited_page(asynchronous))
        direct = asyncio.run(
            afile_list(
                custom_llm_provider="openai",
                purpose="batch",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert dumped(made) == dumped(made_async) == dumped(direct) == PAGE_RECORD
    assert dumped_items(made) == [FILE_RECORD]


def test_error_matches_litellm() -> None:
    with FileCallServer() as server:
        with pytest.raises(openai.NotFoundError) as reference:
            REFERENCE.file_retrieve(
                file_id=MISSING_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(NotFoundError) as made:
            file_retrieve(
                file_id=MISSING_ID,
                custom_llm_provider="openai",
                api_base=server.url,
                api_key=GOOD_KEY,
            )
    assert made.value.status_code == reference.value.status_code == NOT_FOUND
    cause = native_cause(made.value)
    assert cause is made.value.__cause__
    expected: Final[dict[str, JsonValue]] = {
        "code": reference.value.code,
        "type": reference.value.type,
        "param": reference.value.param,
    }
    assert expected == ERROR_DETAILS
    assert ERROR_PAYLOAD.validate_json(str(cause)).get("body") == expected
    # B7870 records the shared exception converter's dropped fields as a deviation.
    assert made.value.code is made.value.type is made.value.param is None


def test_error_token_provider_refused() -> None:
    with pytest.raises(UnsupportedCallArgument, match="azure_ad_token_provider"):
        file_retrieve(file_id=FILE_ID, azure_ad_token_provider=GOOD_KEY)


def test_package_exports() -> None:
    assert tuple(files_package.__all__) == EXPECTED_EXPORTS
    assert tuple(files_main.__all__) == EXPECTED_EXPORTS
    for package_name, main_name in EXPORTED:
        assert package_name is main_name
