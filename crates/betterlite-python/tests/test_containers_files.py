import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Callable, Coroutine, Mapping
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol, TypeAlias
from urllib.parse import parse_qs, urlsplit

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite import containers as container_package
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._router_types_params import GenericLiteLLMParams
from betterlite.containers import _utils
from betterlite.containers._factory import (
    adelete_container_file,
    aretrieve_container_file,
    aretrieve_container_file_content,
    delete_container_file,
    retrieve_container_file,
    retrieve_container_file_content,
)
from betterlite.containers._files import (
    alist_container_files,
    aupload_container_file,
    list_container_files,
    upload_container_file,
)
from betterlite.exceptions import NotFoundError
from betterlite.types.containers.main import (
    ContainerCreateOptionalRequestParams as CreateParams,
)
from betterlite.types.containers.main import (
    ContainerFileListResponse,
    ContainerFileObject,
    DeleteContainerFileResponse,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
NOT_FOUND: Final = 404
JSON_TYPE: Final = "application/json"
BINARY_TYPE: Final = "application/octet-stream"
GOOD_KEY: Final = "sk-test"
API_BASE_PATH: Final = "/v1"
CONTAINER_ID: Final = "cntr_1"
FILE_ID: Final = "file_1"
MISSING_ID: Final = "cntr_missing"
FILES_PATH: Final = f"{API_BASE_PATH}/containers/{CONTAINER_ID}/files"
FILE_PATH: Final = f"{FILES_PATH}/{FILE_ID}"
CONTENT_PATH: Final = f"{FILE_PATH}/content"
AFTER: Final = "cntr_0"
LIMIT: Final = 3
ORDER: Final = "asc"
PAGE_PARAM: Final = "page"
PAGE_VALUE: Final = "2"
PROBE_HEADER: Final = "X-Probe"
PROBE_VALUE: Final = "yes"
CREATED_AT: Final = 1700000000
CSV: Final = b"a,b\n1,2\n"
FILE_NAME: Final = "data.csv"
FILE_KIND: Final = "text/csv"
UPLOAD: Final[tuple[str, bytes, str]] = (FILE_NAME, CSV, FILE_KIND)
NOT_FOUND_TEXT: Final = "No container file found with id 'file_1'"
MODEL_ID: Final = "model_1"

FILE: Final[dict[str, JsonValue]] = {
    "id": FILE_ID,
    "object": "container.file",
    "container_id": CONTAINER_ID,
    "bytes": len(CSV),
    "created_at": CREATED_AT,
    "path": f"/{FILE_NAME}",
    "source": FILE_NAME,
}
FILES: Final[dict[str, JsonValue]] = {
    "object": "list",
    "data": [FILE],
    "first_id": FILE_ID,
    "last_id": FILE_ID,
    "has_more": False,
}
DELETED_FILE: Final[dict[str, JsonValue]] = {
    "id": FILE_ID,
    "object": "container.file.deleted",
    "deleted": True,
}
NOT_FOUND_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": NOT_FOUND_TEXT,
        "type": "invalid_request_error",
        "param": None,
        "code": None,
    }
}

DUMP: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
FileAnswer: TypeAlias = (
    ContainerFileListResponse | ContainerFileObject | DeleteContainerFileResponse | bytes
)
FileCallable: TypeAlias = Callable[..., FileAnswer | Coroutine[JsonValue, JsonValue, FileAnswer]]


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class FileListCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> ContainerFileListResponse: ...


class AsyncFileListCall(Protocol):
    def __call__(
        self, **kwargs: JsonValue
    ) -> Coroutine[JsonValue, JsonValue, ContainerFileListResponse]: ...


class FileCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> ContainerFileObject: ...


class AsyncFileCall(Protocol):
    def __call__(
        self, **kwargs: JsonValue
    ) -> Coroutine[JsonValue, JsonValue, ContainerFileObject]: ...


class DeleteFileCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> DeleteContainerFileResponse: ...


class AsyncDeleteFileCall(Protocol):
    def __call__(
        self, **kwargs: JsonValue
    ) -> Coroutine[JsonValue, JsonValue, DeleteContainerFileResponse]: ...


class ContentCall(Protocol):
    def __call__(self, **kwargs: JsonValue) -> bytes: ...


class AsyncContentCall(Protocol):
    def __call__(
        self, **kwargs: JsonValue
    ) -> Coroutine[JsonValue, JsonValue, bytes]: ...


class FilesReference(Protocol):
    list_container_files: FileListCall
    alist_container_files: AsyncFileListCall
    upload_container_file: FileCall
    aupload_container_file: AsyncFileCall


class FactoryReference(Protocol):
    retrieve_container_file: FileCall
    aretrieve_container_file: AsyncFileCall
    delete_container_file: DeleteFileCall
    adelete_container_file: AsyncDeleteFileCall
    retrieve_container_file_content: ContentCall  # ast-grep-ignore: no-long-names
    aretrieve_container_file_content: AsyncContentCall  # ast-grep-ignore: no-long-names


class PackageReference(Protocol):
    __all__: list[str]


class RequestUtilsReference(Protocol):
    @staticmethod
    def get_requested_container_create_optional_param(  # ast-grep-ignore: no-long-names
        passed_params: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...

    @staticmethod
    def get_requested_container_list_optional_param(  # ast-grep-ignore: no-long-names
        passed_params: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...

    @staticmethod
    def get_optional_params_container_create(  # ast-grep-ignore: no-long-names
        container_provider_config: ContainerConfig,
        container_create_optional_params: CreateParams,  # ast-grep-ignore: no-long-names
    ) -> dict[str, JsonValue]: ...

    @staticmethod
    def encode_container_id_in_response(  # ast-grep-ignore: no-long-names
        response_obj: ContainerFileObject, custom_llm_provider: str | None,
        litellm_metadata: dict[str, JsonValue] | None = None,
        extra_body: dict[str, JsonValue] | None = None,
    ) -> ContainerFileObject: ...


class UtilsReference(Protocol):
    ContainerRequestUtils: type[RequestUtilsReference]

    def decode_managed_container_id_for_request(  # ast-grep-ignore: no-long-names
        self, container_id: str, custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
    ) -> tuple[str, str, GenericLiteLLMParams]: ...


class ContainerConfig(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...

    def map_openai_params(
        self, container_create_optional_params: dict[str, JsonValue],  # ast-grep-ignore: no-long-names
        *, drop_params: bool,
    ) -> dict[str, JsonValue]: ...


FILES_REF: FilesReference = importlib.import_module("litellm.containers.main")
FACTORY_REF: FactoryReference = importlib.import_module("litellm.containers.endpoint_factory")
PACKAGE_REF: PackageReference = importlib.import_module("litellm.containers")
UTILS_REF: UtilsReference = importlib.import_module("litellm.containers.utils")

SIGNATURE_PAIRS: Final[list[tuple[FileCallable, FileCallable, str]]] = [
    (list_container_files, FILES_REF.list_container_files, "sync"),
    (alist_container_files, FILES_REF.alist_container_files, "async"),
    (upload_container_file, FILES_REF.upload_container_file, "sync"),
    (aupload_container_file, FILES_REF.aupload_container_file, "async"),
    (retrieve_container_file, FACTORY_REF.retrieve_container_file, "sync"),
    (aretrieve_container_file, FACTORY_REF.aretrieve_container_file, "async"),
    (delete_container_file, FACTORY_REF.delete_container_file, "sync"),
    (adelete_container_file, FACTORY_REF.adelete_container_file, "async"),
    (retrieve_container_file_content,  # ast-grep-ignore: no-long-names
     FACTORY_REF.retrieve_container_file_content, "sync"),
    (aretrieve_container_file_content,  # ast-grep-ignore: no-long-names
     FACTORY_REF.aretrieve_container_file_content, "async"),
]


class FakeContainerConfig:
    def get_supported_openai_params(self) -> list[str]:
        return [
            "expires_after", "file_ids",
        ]

    def map_openai_params(
        self, container_create_optional_params: Mapping[str, JsonValue],  # ast-grep-ignore: no-long-names
        *, drop_params: bool,
    ) -> dict[str, JsonValue]:
        mapped = dict(container_create_optional_params)  # ast-grep-ignore: no-long-names
        mapped["drop_params"] = drop_params
        return mapped


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class FileServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), FileHandler)
        self.calls = []
        FileHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def answer(self, handler: BaseHTTPRequestHandler) -> None:
        self.calls.append(recorded(handler, read_body(handler)))
        status, content, kind = route_of(handler.command, handler.path)
        handler.send_response(status)
        handler.send_header("Content-Type", kind)
        handler.send_header("Content-Length", str(len(content)))
        handler.end_headers()
        handler.wfile.write(content)

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class FileHandler(BaseHTTPRequestHandler):
    owner: ClassVar[FileServer]

    def do_POST(self) -> None:
        self.owner.answer(self)

    def do_GET(self) -> None:
        self.owner.answer(self)

    def do_DELETE(self) -> None:
        self.owner.answer(self)


def read_body(handler: BaseHTTPRequestHandler) -> bytes:
    declared = handler.headers.get("Content-Length")
    return b"" if declared is None else handler.rfile.read(int(declared))


def recorded(handler: BaseHTTPRequestHandler, body: bytes) -> RecordedCall:
    headers = dict(handler.headers.items())
    return {"method": handler.command, "path": handler.path, "headers": headers, "body": body}


def json_bytes(payload: JsonValue) -> bytes:
    text = json.dumps(payload)
    return text.encode()


def route_of(method: str, target: str) -> tuple[int, bytes, str]:
    path = urlsplit(target).path
    if MISSING_ID in path:
        return NOT_FOUND, json_bytes(NOT_FOUND_BODY), JSON_TYPE
    if path == CONTENT_PATH:
        return OK, CSV, BINARY_TYPE
    if path == FILES_PATH and method == "POST":
        return OK, json_bytes(FILE), JSON_TYPE
    if method == "DELETE":
        return OK, json_bytes(DELETED_FILE), JSON_TYPE
    if path == FILES_PATH:
        return OK, json_bytes(FILES), JSON_TYPE
    return OK, json_bytes(FILE), JSON_TYPE


def rows_of(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        view: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(view.default)))
    return rows


def dumped_of(
    answer: ContainerFileObject | ContainerFileListResponse | DeleteContainerFileResponse,
) -> dict[str, JsonValue]:
    encoded = answer.model_dump_json()
    return DUMP.validate_json(encoded)


def query_of(call: RecordedCall) -> dict[str, str]:
    parsed = parse_qs(urlsplit(call["path"]).query)
    return {name: values[0] for name, values in parsed.items()}


def header_of(call: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def paths_of(calls: list[RecordedCall]) -> list[str]:
    return [
        urlsplit(call["path"]).path for call in calls
    ]


@pytest.mark.parametrize(("ours", "reference", "kind"), SIGNATURE_PAIRS)
def test_signature_matches_litellm(
    ours: FileCallable, reference: FileCallable, kind: str,
) -> None:
    assert rows_of(inspect.signature(ours)) == rows_of(inspect.signature(reference))
    assert inspect.iscoroutinefunction(ours) is (kind == "async")


def test_signature_package_exports() -> None:
    expected = set(PACKAGE_REF.__all__) | {"upload_container_file", "aupload_container_file"}
    assert set(container_package.__all__) == expected


def test_calls_list_files() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = FILES_REF.list_container_files(
            container_id=CONTAINER_ID, after=AFTER, limit=LIMIT, order=ORDER,
            api_base=base, api_key=GOOD_KEY,
        )
        ours = list_container_files(
            container_id=CONTAINER_ID, after=AFTER, limit=LIMIT, order=ORDER,
            api_base=base, api_key=GOOD_KEY,
        )
        async_reference = asyncio.run(FILES_REF.alist_container_files(
            container_id=CONTAINER_ID, after=AFTER, limit=LIMIT, order=ORDER,
            api_base=base, api_key=GOOD_KEY,
        ))
        async_ours = asyncio.run(alist_container_files(
            container_id=CONTAINER_ID, after=AFTER, limit=LIMIT, order=ORDER,
            api_base=base, api_key=GOOD_KEY,
        ))
        calls = list(server.calls)
    assert dumped_of(ours) == dumped_of(reference)
    assert dumped_of(async_ours) == dumped_of(async_reference)
    assert dumped_of(ours) == dumped_of(async_ours)
    assert paths_of(calls) == [FILES_PATH] * 4
    assert [query_of(call) for call in calls] == [
        {"after": AFTER, "limit": str(LIMIT), "order": ORDER}
    ] * 4
    assert header_of(calls[1], "authorization") == f"Bearer {GOOD_KEY}"


def test_calls_upload_file() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = FILES_REF.upload_container_file(
            container_id=CONTAINER_ID, file=UPLOAD, api_base=base, api_key=GOOD_KEY,
        )
        ours = upload_container_file(
            container_id=CONTAINER_ID, file=UPLOAD, api_base=base, api_key=GOOD_KEY,
        )
        async_ours = asyncio.run(aupload_container_file(
            container_id=CONTAINER_ID, file=UPLOAD, api_base=base, api_key=GOOD_KEY,
        ))
        calls = list(server.calls)
    assert dumped_of(ours) == dumped_of(reference)
    assert dumped_of(async_ours) == dumped_of(reference)
    assert paths_of(calls) == [FILES_PATH] * 3
    assert {call["method"] for call in calls} == {"POST"}
    for call in calls:
        kind = header_of(call, "content-type")
        assert kind is not None
        assert kind.startswith("multipart/form-data; boundary=")
        assert b'name="file"' in call["body"]
        assert CSV in call["body"]
    for call in calls[1:]:
        assert f'filename="{FILE_NAME}"'.encode() in call["body"]
        assert FILE_KIND.encode() in call["body"]


def test_calls_retrieve_and_delete() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = FACTORY_REF.retrieve_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        )
        ours = retrieve_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        )
        reference_deleted = FACTORY_REF.delete_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        )
        ours_deleted = delete_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        )
        async_ours = asyncio.run(aretrieve_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        ))
        async_deleted = asyncio.run(adelete_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        ))
        calls = list(server.calls)
    assert dumped_of(ours) == dumped_of(reference)
    assert dumped_of(async_ours) == dumped_of(reference)
    assert dumped_of(ours_deleted) == dumped_of(reference_deleted)
    assert dumped_of(async_deleted) == dumped_of(reference_deleted)
    assert paths_of(calls) == [FILE_PATH] * 6
    assert [call["method"] for call in calls] == ["GET", "GET", "DELETE", "DELETE", "GET", "DELETE"]


def test_calls_content_bytes() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = FACTORY_REF.retrieve_container_file_content(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        )
        ours = retrieve_container_file_content(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        )
        async_ours = asyncio.run(aretrieve_container_file_content(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
        ))
        calls = list(server.calls)
    assert ours == CSV
    assert reference == CSV
    assert async_ours == CSV
    assert paths_of(calls) == [CONTENT_PATH] * 3


def test_calls_extra_params() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        sent_headers: dict[str, JsonValue] = {PROBE_HEADER: PROBE_VALUE}
        sent_query: dict[str, JsonValue] = {PAGE_PARAM: PAGE_VALUE}
        ours = retrieve_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
            extra_headers=sent_headers, extra_query=sent_query,
        )
        reference = FACTORY_REF.retrieve_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
            extra_headers=sent_headers, extra_query=sent_query,
        )
        [ours_call, reference_call] = server.calls
    assert dumped_of(ours) == dumped_of(reference)
    assert query_of(ours_call) == query_of(reference_call)
    assert query_of(ours_call) == {PAGE_PARAM: PAGE_VALUE}
    assert header_of(ours_call, PROBE_HEADER) == header_of(reference_call, PROBE_HEADER)
    assert header_of(ours_call, PROBE_HEADER) == PROBE_VALUE


def test_calls_utils_params() -> None:
    given: dict[str, JsonValue] = {
        "expires_after": {"anchor": "last_active_at", "minutes": 30},
        "file_ids": ["file_1"],
        "name": "probe",
    }
    listed: dict[str, JsonValue] = {
        "after": AFTER, "limit": LIMIT, "order": ORDER, "timeout": 4,
    }
    ours = _utils.get_requested_container_create_optional_param(given)
    reference = UTILS_REF.ContainerRequestUtils.get_requested_container_create_optional_param(given)
    assert dict(ours) == dict(reference)
    ours_listed = _utils.get_requested_container_list_optional_param(listed)
    reference_listed = UTILS_REF.ContainerRequestUtils.get_requested_container_list_optional_param(
        listed
    )
    assert dict(ours_listed) == dict(reference_listed)
    mapped = _utils.get_optional_params_container_create(FakeContainerConfig(), ours)
    reference_mapped = UTILS_REF.ContainerRequestUtils.get_optional_params_container_create(
        FakeContainerConfig(), ours
    )
    assert mapped == reference_mapped
    assert sorted(mapped) == ["drop_params", "expires_after", "file_ids"]


def test_calls_utils_codec() -> None:
    ours = ContainerFileObject.model_validate(FILE)
    reference = ContainerFileObject.model_validate(FILE)
    metadata: dict[str, JsonValue] = {"model_info": {"id": MODEL_ID}}
    encoded_ours = _utils.encode_container_id_in_response(ours, "openai", metadata, None)
    encoded_reference = UTILS_REF.ContainerRequestUtils.encode_container_id_in_response(
        reference, "openai", metadata, None
    )
    assert encoded_ours.id == encoded_reference.id
    targets: dict[str, JsonValue] = {"target_model_names": f"{MODEL_ID},fallback"}
    routed = _utils.encode_container_id_in_response(
        ContainerFileObject.model_validate(FILE), "openai", None, targets
    )
    routed_reference = UTILS_REF.ContainerRequestUtils.encode_container_id_in_response(
        ContainerFileObject.model_validate(FILE), "openai", None, targets
    )
    assert routed.id == routed_reference.id
    untouched = _utils.encode_container_id_in_response(
        ContainerFileObject.model_validate(FILE), "openai", None, None
    )
    assert untouched.id == FILE_ID
    for container_id in (
        encoded_reference.id, FILE_ID, "cntr_not_base64", "cntr_bGl0ZWxsaW06c3R1ZmY=",
    ):
        ours_params = GenericLiteLLMParams()
        reference_params = GenericLiteLLMParams()
        ours_decoded = _utils.decode_managed_container_id_for_request(
            container_id, "openai", ours_params
        )
        reference_decoded = UTILS_REF.decode_managed_container_id_for_request(
            container_id=container_id, custom_llm_provider="openai",
            litellm_params=reference_params,
        )
        assert ours_decoded[0] == reference_decoded[0]
        assert ours_decoded[1] == reference_decoded[1]
        assert ours_decoded[2].get("model_id") == reference_decoded[2].get("model_id")


def test_error_retrieve_missing() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        with pytest.raises(NotFoundError) as caught:
            retrieve_container_file(
                container_id=MISSING_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
            )
        with pytest.raises(Exception, match=NOT_FOUND_TEXT) as reference_caught:
            FACTORY_REF.retrieve_container_file(
                container_id=MISSING_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
            )
    error = caught.value
    assert error.status_code == NOT_FOUND
    assert error.__class__.__name__ == reference_caught.value.__class__.__name__
    assert NOT_FOUND_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeContainerFileError"


def test_error_async_missing() -> None:
    with closing(FileServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        with pytest.raises(NotFoundError) as caught:
            asyncio.run(aretrieve_container_file_content(
                container_id=MISSING_ID, file_id=FILE_ID, api_base=base, api_key=GOOD_KEY,
            ))
    assert caught.value.status_code == NOT_FOUND
    assert NOT_FOUND_TEXT in caught.value.message


def test_unsupported_client() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        list_container_files(container_id=CONTAINER_ID, client=object())
    assert caught.value.argument == "client"


def test_unsupported_session() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        retrieve_container_file(
            container_id=CONTAINER_ID, file_id=FILE_ID, shared_session=object()
        )
    assert caught.value.argument == "shared_session"


def test_unsupported_async_token() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        asyncio.run(aupload_container_file(
            container_id=CONTAINER_ID, file=CSV, azure_ad_token_provider=object()
        ))
    assert caught.value.argument == "azure_ad_token_provider"
