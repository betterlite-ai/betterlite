import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Awaitable, Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import TracebackType
from typing import ClassVar, Final, NamedTuple, Protocol, Self, TypeAlias, runtime_checkable
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

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
NOT_FOUND: Final = 404
GOOD_KEY: Final = "sk-test"
FILE_ID: Final = "file-abc123"
MISSING_ID: Final = "file-missing"
FILES_PATH: Final = "/v1/files"
FILE_PATH: Final = f"{FILES_PATH}/{FILE_ID}"
LIST_PATH: Final = f"{FILES_PATH}?purpose=batch"
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
FILE_RECORD: Final[dict[str, JsonValue]] = {
    **SERVER_FILE,
    "expires_at": None,
    "status_details": None,
}
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


@runtime_checkable
class ReferenceFile(Protocol):
    def model_dump_json(self) -> str: ...


@runtime_checkable
class ReferencePage(ReferenceFile, Protocol):
    def __iter__(self) -> Iterator[ReferenceFile]: ...


CallResult: TypeAlias = ReferenceFile | Awaitable[ReferenceFile]


# every reference call is dumped as a file record, and a page is a record whose rows iterate
class ReferenceCall(Protocol):
    def __call__(self, *args: CallValue, **kwargs: CallValue) -> CallResult: ...


class FilesModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


REFERENCE: FilesModule = importlib.import_module("litellm")  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/files/main.py#L115-L804
OURS: FilesModule = importlib.import_module("betterlite.files.main")


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


def dumped(value: CallResult) -> dict[str, JsonValue]:
    match value:
        case ReferenceFile():
            return JSON_RECORD.validate_json(value.model_dump_json())
        case Awaitable():
            raise AssertionError("the call answered with a coroutine instead of a record")


def dumped_items(value: CallResult) -> list[dict[str, JsonValue]]:
    match value:
        case ReferencePage():
            return [dumped(item) for item in value]
        case ReferenceFile() | Awaitable():
            raise AssertionError("the call did not answer with a file page")


# the record either call answered with, awaited when the call handed back its coroutine
async def resolved(value: CallResult) -> ReferenceFile:
    match value:
        case Awaitable():
            return await value
        case ReferenceFile():
            return value


def native_cause(error: BaseException) -> NativeFileCallError:
    cause = error.__cause__
    match cause:
        case NativeFileCallError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


class Comparison(NamedTuple):
    ours: CallResult
    reference: CallResult
    ours_call: RecordedCall
    reference_call: RecordedCall


def endpoint_values(server: FileCallServer, **arguments: CallValue) -> dict[str, CallValue]:
    values: dict[str, CallValue] = {"custom_llm_provider": "openai", "api_base": server.url}
    values["api_key"] = GOOD_KEY
    values.update(arguments)
    return values


# the reference call first, then ours against the same server and kwargs, keeping both calls
def compared(
    server: FileCallServer,
    ours: ReferenceCall,
    reference: ReferenceCall,
    **arguments: CallValue,
) -> Comparison:
    values = endpoint_values(server, **arguments)
    reference_value = reference(**values)
    reference_call = server.last()
    server.calls.clear()
    ours_value = ours(**values)
    return Comparison(ours_value, reference_value, server.last(), reference_call)


def assert_same_call(
    reference_call: RecordedCall, ours_call: RecordedCall, method: str, path: str,
) -> None:
    assert ours_call["method"] == reference_call["method"] == method
    assert ours_call["path"] == reference_call["path"] == path


def compared_upload(
    server: FileCallServer, source: CallValue,
) -> tuple[Comparison, dict[str, Part], dict[str, Part]]:
    swapped = compared(
        server, OURS.create_file, REFERENCE.create_file, file=source, purpose="batch")
    reference_parts = sent_parts(swapped.reference_call)
    return swapped, reference_parts, sent_parts(swapped.ours_call)


def assert_same_upload(
    swapped: Comparison, reference_parts: dict[str, Part], ours_parts: dict[str, Part],
    filename: str,
) -> None:
    assert_same_call(swapped.reference_call, swapped.ours_call, "POST", FILES_PATH)
    assert ours_parts["file"]["filename"] == reference_parts["file"]["filename"] == filename
    assert ours_parts["file"]["body"] == reference_parts["file"]["body"] == PAYLOAD
    assert ours_parts["purpose"]["body"] == reference_parts["purpose"]["body"] == b"batch"
    assert dumped(swapped.ours) == dumped(swapped.reference) == FILE_RECORD


# the sync call under its a-name flag and the async twin itself, both awaited
def flagged_values(
    server: FileCallServer,
    synchronous: ReferenceCall,
    asynchronous: ReferenceCall,
    flag: str,
    **arguments: CallValue,
) -> list[ReferenceFile]:
    values = endpoint_values(server, **arguments)
    flagged = synchronous(**values, **{flag: True})
    direct = asynchronous(**values)
    assert inspect.isawaitable(flagged)
    return [asyncio.run(resolved(flagged)), asyncio.run(resolved(direct))]


def async_mode_value(
    server: FileCallServer, synchronous: ReferenceCall, **arguments: CallValue
) -> ReferenceFile:
    values = endpoint_values(server, **arguments)
    flagged = synchronous(**values, is_async=True)
    assert inspect.isawaitable(flagged)
    return asyncio.run(resolved(flagged))


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(create_file)) == parameter_rows(
        inspect.signature(REFERENCE.create_file))
    assert parameter_rows(inspect.signature(acreate_file)) == parameter_rows(
        inspect.signature(REFERENCE.acreate_file))
    assert parameter_rows(inspect.signature(file_retrieve)) == parameter_rows(
        inspect.signature(REFERENCE.file_retrieve))
    assert parameter_rows(inspect.signature(afile_retrieve)) == parameter_rows(
        inspect.signature(REFERENCE.afile_retrieve))
    assert parameter_rows(inspect.signature(file_delete)) == parameter_rows(
        inspect.signature(REFERENCE.file_delete))
    assert parameter_rows(inspect.signature(afile_delete)) == parameter_rows(
        inspect.signature(REFERENCE.afile_delete))
    assert parameter_rows(inspect.signature(file_list)) == parameter_rows(
        inspect.signature(REFERENCE.file_list))
    assert parameter_rows(inspect.signature(afile_list)) == parameter_rows(
        inspect.signature(REFERENCE.afile_list))
    assert not inspect.iscoroutinefunction(create_file)
    assert not inspect.iscoroutinefunction(file_retrieve)
    assert not inspect.iscoroutinefunction(file_delete)
    assert not inspect.iscoroutinefunction(file_list)
    assert inspect.iscoroutinefunction(acreate_file)
    assert inspect.iscoroutinefunction(afile_retrieve)
    assert inspect.iscoroutinefunction(afile_delete)
    assert inspect.iscoroutinefunction(afile_list)


def test_create_bytes_matches_litellm() -> None:  # ast-grep-ignore: no-long-names
    with FileCallServer() as server:
        swapped, reference_parts, ours_parts = compared_upload(server, PAYLOAD)
    assert_same_upload(swapped, reference_parts, ours_parts, UPLOAD_NAME)


def test_create_path_matches_litellm(tmp_path: Path) -> None:  # ast-grep-ignore: no-long-names
    rows = tmp_path / CREATE_FILENAME
    rows.write_bytes(PAYLOAD)
    with FileCallServer() as server:
        swapped, reference_parts, ours_parts = compared_upload(server, rows)
    assert_same_upload(swapped, reference_parts, ours_parts, CREATE_FILENAME)


def test_create_file_object_matches_litellm(  # ast-grep-ignore: no-long-names
    tmp_path: Path,
) -> None:
    rows = tmp_path / CREATE_FILENAME
    rows.write_bytes(PAYLOAD)
    with FileCallServer() as server, rows.open("rb") as stream:
        reference = REFERENCE.create_file(
            file=stream, purpose="batch", custom_llm_provider="openai",
            api_base=server.url, api_key=GOOD_KEY,
        )
        reference_call = server.last()
        server.calls.clear()
        stream.seek(0)
        position = stream.tell()
        ours = OURS.create_file(
            file=stream, purpose="batch", custom_llm_provider="openai",
            api_base=server.url, api_key=GOOD_KEY,
        )
        assert stream.tell() == position
        ours_call = server.last()
    swapped = Comparison(ours, reference, ours_call, reference_call)
    assert_same_upload(
        swapped, sent_parts(reference_call), sent_parts(ours_call), CREATE_FILENAME)


def test_create_tuple_matches_litellm() -> None:  # ast-grep-ignore: no-long-names
    with FileCallServer() as server:
        swapped, reference_parts, ours_parts = compared_upload(server, (TUPLE_FILENAME, PAYLOAD))
    assert_same_upload(swapped, reference_parts, ours_parts, TUPLE_FILENAME)


def test_create_expires_after_reaches_server() -> None:  # ast-grep-ignore: no-long-names
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
        values = flagged_values(
            server, OURS.create_file, OURS.acreate_file, "acreate_file",
            file=PAYLOAD, purpose="batch",
        )
    assert [dumped(value) for value in values] == [FILE_RECORD, FILE_RECORD]


def test_create_provider_none_matches_litellm() -> None:  # ast-grep-ignore: no-long-names
    with FileCallServer() as server:
        with pytest.raises(ValueError, match="is not a valid LlmProviders") as reference:
            REFERENCE.create_file(
                file=PAYLOAD,
                purpose="batch",
                custom_llm_provider=None,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        with pytest.raises(ValueError, match="is not a valid LlmProviders"):
            create_file(
                file=PAYLOAD,
                purpose="batch",
                custom_llm_provider=None,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
    assert "None" in str(reference.value)


def test_retrieve_matches_litellm() -> None:
    with FileCallServer() as server:
        swapped = compared(server, OURS.file_retrieve, REFERENCE.file_retrieve, file_id=FILE_ID)
    assert_same_call(swapped.reference_call, swapped.ours_call, "GET", FILE_PATH)
    assert dumped(swapped.ours) == dumped(swapped.reference) == FILE_RECORD


def test_retrieve_flag_coroutine() -> None:
    with FileCallServer() as server:
        values = flagged_values(
            server, OURS.file_retrieve, OURS.afile_retrieve, "afile_retrieve", file_id=FILE_ID)
        asynchronous = async_mode_value(server, OURS.file_retrieve, file_id=FILE_ID)
    assert [dumped(value) for value in values] == [FILE_RECORD, FILE_RECORD]
    assert dumped(asynchronous) == FILE_RECORD


def test_delete_matches_litellm() -> None:
    with FileCallServer() as server:
        swapped = compared(
            server, OURS.file_delete, REFERENCE.file_delete, file_id=FILE_ID, model="gpt-4o")
    assert_same_call(swapped.reference_call, swapped.ours_call, "DELETE", FILE_PATH)
    assert dumped(swapped.ours) == dumped(swapped.reference) == DELETED_RECORD


def test_delete_flag_coroutine() -> None:
    with FileCallServer() as server:
        values = flagged_values(
            server, OURS.file_delete, OURS.afile_delete, "afile_delete",
            file_id=FILE_ID, model="gpt-4o",
        )
        asynchronous = async_mode_value(
            server, OURS.file_delete, file_id=FILE_ID, model="gpt-4o")
    assert [dumped(value) for value in values] == [DELETED_RECORD, DELETED_RECORD]
    assert dumped(asynchronous) == DELETED_RECORD


def test_list_matches_litellm() -> None:
    with FileCallServer() as server:
        swapped = compared(server, OURS.file_list, REFERENCE.file_list, purpose="batch")
    assert_same_call(swapped.reference_call, swapped.ours_call, "GET", LIST_PATH)
    assert dumped(swapped.ours) == dumped(swapped.reference) == PAGE_RECORD
    assert dumped_items(swapped.ours) == dumped_items(swapped.reference) == [FILE_RECORD]
    assert dumped(swapped.ours)["has_more"] is False


def test_list_without_purpose_matches_litellm() -> None:  # ast-grep-ignore: no-long-names
    with FileCallServer() as server:
        swapped = compared(server, OURS.file_list, REFERENCE.file_list)
    assert_same_call(swapped.reference_call, swapped.ours_call, "GET", FILES_PATH)
    assert dumped(swapped.ours) == dumped(swapped.reference) == PAGE_RECORD


def test_list_flag_coroutine() -> None:
    with FileCallServer() as server:
        values = flagged_values(
            server, OURS.file_list, OURS.afile_list, "afile_list", purpose="batch")
        asynchronous = async_mode_value(server, OURS.file_list, purpose="batch")
    assert [dumped(value) for value in values] == [PAGE_RECORD, PAGE_RECORD]
    assert dumped_items(values[0]) == [FILE_RECORD]
    assert dumped(asynchronous) == PAGE_RECORD


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


def test_error_token_provider_refused() -> None:  # ast-grep-ignore: no-long-names
    with pytest.raises(UnsupportedCallArgument, match="azure_ad_token_provider"):
        file_retrieve(file_id=FILE_ID, azure_ad_token_provider=GOOD_KEY)


def test_package_exports() -> None:
    assert tuple(files_package.__all__) == EXPECTED_EXPORTS
    assert tuple(files_main.__all__) == EXPECTED_EXPORTS
    assert files_package.FileContentStreamingResponse is files_main.FileContentStreamingResponse
    assert files_package.acreate_file is files_main.acreate_file
    assert files_package.afile_content is files_main.afile_content
    assert files_package.afile_delete is files_main.afile_delete
    assert files_package.afile_list is files_main.afile_list
    assert files_package.afile_retrieve is files_main.afile_retrieve
    assert files_package.create_file is files_main.create_file
    assert files_package.file_content is files_main.file_content
    assert files_package.file_delete is files_main.file_delete
    assert files_package.file_list is files_main.file_list
    assert files_package.file_retrieve is files_main.file_retrieve
