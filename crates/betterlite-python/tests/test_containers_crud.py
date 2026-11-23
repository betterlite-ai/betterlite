import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Coroutine
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol
from typing_extensions import TypedDict
from urllib.parse import parse_qs, urlsplit

import pytest
from pydantic import JsonValue, TypeAdapter

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.containers._crud import (
    acreate_container,
    adelete_container,
    alist_containers,
    aretrieve_container,
    create_container,
    delete_container,
    list_containers,
    retrieve_container,
)
from betterlite.exceptions import NotFoundError
from betterlite.types.containers.main import (
    ContainerListResponse,
    ContainerObject,
    DeleteContainerResult,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
NOT_FOUND: Final = 404
JSON_TYPE: Final = "application/json"
GOOD_KEY: Final = "sk-test"
API_BASE_PATH: Final = "/v1"
CONTAINERS_PATH: Final = "/v1/containers"
CONTAINER_ID: Final = "cntr_1"
MISSING_ID: Final = "cntr_missing"
MISSING_PATH: Final = f"{CONTAINERS_PATH}/{MISSING_ID}"
CONTAINER_NAME: Final = "probe"
COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
CREATED_AT: Final = 1700000000
LIMIT: Final = 3
AFTER: Final = "cntr_0"
ORDER: Final = "asc"
PAGE_PARAM: Final = "page"
PAGE_VALUE: Final = "2"
PROBE_HEADER: Final = "X-Probe"
PROBE_VALUE: Final = "yes"
EXPIRES_AFTER: Final[dict[str, JsonValue]] = {"anchor": "last_active_at", "minutes": 20}
FILE_IDS: Final[list[JsonValue]] = ["file_1", "file_2"]
OUR_FILE_IDS: Final[list[str]] = ["file_1", "file_2"]
NOT_FOUND_TEXT: Final = "No container found with id 'cntr_missing'"
CONTAINER: Final[dict[str, JsonValue]] = {
    "id": CONTAINER_ID,
    "object": "container",
    "created_at": CREATED_AT,
    "status": "running",
    "expires_after": EXPIRES_AFTER,
    "last_active_at": CREATED_AT,
    "name": CONTAINER_NAME,
}
CONTAINERS: Final[dict[str, JsonValue]] = {
    "object": "list",
    "data": [CONTAINER],
    "first_id": CONTAINER_ID,
    "last_id": CONTAINER_ID,
    "has_more": False,
}
DELETED: Final[dict[str, JsonValue]] = {
    "id": CONTAINER_ID,
    "object": "container.deleted",
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
FLOAT: Final[TypeAdapter[float]] = TypeAdapter(float)


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class ReferenceContainer(Protocol):
    def __call__(self, **kwargs: JsonValue) -> ContainerObject: ...


class ReferenceAsyncContainer(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Coroutine[JsonValue, JsonValue, ContainerObject]: ...


class ReferenceList(Protocol):
    def __call__(self, **kwargs: JsonValue) -> ContainerListResponse: ...


class ReferenceAsyncList(Protocol):
    def __call__(
        self, **kwargs: JsonValue
    ) -> Coroutine[JsonValue, JsonValue, ContainerListResponse]: ...


class ReferenceDelete(Protocol):
    def __call__(self, **kwargs: JsonValue) -> DeleteContainerResult: ...


class ReferenceAsyncDelete(Protocol):
    def __call__(
        self, **kwargs: JsonValue
    ) -> Coroutine[JsonValue, JsonValue, DeleteContainerResult]: ...


class ReferenceModule(Protocol):
    create_container: ReferenceContainer
    acreate_container: ReferenceAsyncContainer
    list_containers: ReferenceList
    alist_containers: ReferenceAsyncList
    retrieve_container: ReferenceContainer
    aretrieve_container: ReferenceAsyncContainer
    delete_container: ReferenceDelete
    adelete_container: ReferenceAsyncDelete


REFERENCE: ReferenceModule = importlib.import_module("litellm")


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class ContainerServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), ContainerHandler)
        self.calls = []
        ContainerHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    # one answer table for every route, and the request kept so a test can read what was sent
    def answer(self, handler: BaseHTTPRequestHandler) -> None:
        declared = handler.headers.get("Content-Length")
        body = b"" if declared is None else handler.rfile.read(int(declared))
        self.calls.append({
            "method": handler.command,
            "path": handler.path,
            "headers": dict(handler.headers.items()),
            "body": body,
        })
        status, payload = route_of(handler.command, handler.path)
        content = json.dumps(payload).encode()
        handler.send_response(status)
        handler.send_header("Content-Type", JSON_TYPE)
        handler.send_header("Content-Length", str(len(content)))
        handler.end_headers()
        handler.wfile.write(content)

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class ContainerHandler(BaseHTTPRequestHandler):
    owner: ClassVar[ContainerServer]

    def do_POST(self) -> None:
        self.owner.answer(self)

    def do_GET(self) -> None:
        self.owner.answer(self)

    def do_DELETE(self) -> None:
        self.owner.answer(self)


def route_of(method: str, target: str) -> tuple[int, dict[str, JsonValue]]:
    path = urlsplit(target).path
    if path == CONTAINERS_PATH:
        return (OK, CONTAINER) if method == "POST" else (OK, CONTAINERS)
    if path == MISSING_PATH:
        return NOT_FOUND, NOT_FOUND_BODY
    if method == "DELETE":
        return OK, DELETED
    return OK, CONTAINER


def rows_of(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        view: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(view.default)))
    return rows


def dumped_of(
    response: ContainerObject | ContainerListResponse | DeleteContainerResult,
) -> dict[str, JsonValue]:
    dumped = response.model_dump()
    return DUMP.validate_python(dumped)


def hidden_of(container: ContainerObject) -> dict[str, JsonValue]:
    private: dict[str, JsonValue] | None = container.__pydantic_private__
    assert private is not None
    return DUMP.validate_python(private["_hidden_params"])


def body_of(call: RecordedCall) -> dict[str, JsonValue]:
    recorded = call["body"]
    return DUMP.validate_json(recorded)


def query_of(call: RecordedCall) -> dict[str, str]:
    parsed = parse_qs(urlsplit(call["path"]).query)
    return {name: values[0] for name, values in parsed.items()}


def header_of(call: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def cost_of(container: ContainerObject) -> float:
    headers = DUMP.validate_python(hidden_of(container)["additional_headers"])
    return FLOAT.validate_python(headers[COST_HEADER])


def test_signature_matches_litellm() -> None:
    assert rows_of(inspect.signature(create_container)) == rows_of(
        inspect.signature(REFERENCE.create_container)
    )
    assert rows_of(inspect.signature(acreate_container)) == rows_of(
        inspect.signature(REFERENCE.acreate_container)
    )
    assert rows_of(inspect.signature(list_containers)) == rows_of(
        inspect.signature(REFERENCE.list_containers)
    )
    assert rows_of(inspect.signature(alist_containers)) == rows_of(
        inspect.signature(REFERENCE.alist_containers)
    )
    assert rows_of(inspect.signature(retrieve_container)) == rows_of(
        inspect.signature(REFERENCE.retrieve_container)
    )
    assert rows_of(inspect.signature(aretrieve_container)) == rows_of(
        inspect.signature(REFERENCE.aretrieve_container)
    )
    assert rows_of(inspect.signature(delete_container)) == rows_of(
        inspect.signature(REFERENCE.delete_container)
    )
    assert rows_of(inspect.signature(adelete_container)) == rows_of(
        inspect.signature(REFERENCE.adelete_container)
    )
    assert not inspect.iscoroutinefunction(create_container)
    assert not inspect.iscoroutinefunction(list_containers)
    assert inspect.iscoroutinefunction(acreate_container)
    assert inspect.iscoroutinefunction(alist_containers)
    assert inspect.iscoroutinefunction(aretrieve_container)
    assert inspect.iscoroutinefunction(adelete_container)
    # create names its container, and both packages refuse a call without one
    with pytest.raises(TypeError):
        inspect.signature(create_container).bind()
    with pytest.raises(TypeError):
        inspect.signature(REFERENCE.create_container).bind()


def test_calls_create() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = REFERENCE.create_container(
            name=CONTAINER_NAME,
            expires_after=EXPIRES_AFTER,
            file_ids=FILE_IDS,
            api_base=base,
            api_key=GOOD_KEY,
        )
        ours = create_container(
            name=CONTAINER_NAME,
            expires_after=EXPIRES_AFTER,
            file_ids=OUR_FILE_IDS,
            api_base=base,
            api_key=GOOD_KEY,
        )
        async_reference = asyncio.run(
            REFERENCE.acreate_container(
                name=CONTAINER_NAME,
                expires_after=EXPIRES_AFTER,
                file_ids=FILE_IDS,
                api_base=base,
                api_key=GOOD_KEY,
            )
        )
        async_ours = asyncio.run(
            acreate_container(
                name=CONTAINER_NAME,
                expires_after=EXPIRES_AFTER,
                file_ids=OUR_FILE_IDS,
                api_base=base,
                api_key=GOOD_KEY,
            )
        )
        [sync_reference_call, sync_call, async_reference_call, async_call] = server.calls
    assert dumped_of(ours) == dumped_of(reference)
    assert dumped_of(async_ours) == dumped_of(async_reference)
    assert dumped_of(ours) == dumped_of(async_ours)
    assert body_of(sync_call) == body_of(sync_reference_call)
    assert body_of(async_call) == body_of(async_reference_call)
    assert body_of(sync_call) == {
        "name": CONTAINER_NAME,
        "expires_after": EXPIRES_AFTER,
        "file_ids": FILE_IDS,
    }
    assert header_of(sync_call, "authorization") == f"Bearer {GOOD_KEY}"
    assert header_of(sync_reference_call, "authorization") == f"Bearer {GOOD_KEY}"
    assert cost_of(ours) == pytest.approx(cost_of(reference))
    assert cost_of(async_ours) == pytest.approx(cost_of(async_reference))


def test_calls_list() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = REFERENCE.list_containers(
            after=AFTER, limit=LIMIT, order=ORDER, api_base=base, api_key=GOOD_KEY
        )
        ours = list_containers(
            after=AFTER, limit=LIMIT, order=ORDER, api_base=base, api_key=GOOD_KEY
        )
        async_reference = asyncio.run(
            REFERENCE.alist_containers(
                after=AFTER, limit=LIMIT, order=ORDER, api_base=base, api_key=GOOD_KEY
            )
        )
        async_ours = asyncio.run(
            alist_containers(
                after=AFTER, limit=LIMIT, order=ORDER, api_base=base, api_key=GOOD_KEY
            )
        )
        recorded = list(server.calls)
    assert dumped_of(ours) == dumped_of(reference)
    assert dumped_of(async_ours) == dumped_of(async_reference)
    assert query_of(recorded[0]) == {"after": AFTER, "limit": str(LIMIT), "order": ORDER}
    assert query_of(recorded[1]) == query_of(recorded[0])
    assert query_of(recorded[2]) == query_of(recorded[0])
    assert query_of(recorded[3]) == query_of(recorded[0])


def test_calls_list_defaults() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = REFERENCE.list_containers(api_base=base, api_key=GOOD_KEY)
        ours = list_containers(api_base=base, api_key=GOOD_KEY)
        [reference_call, call] = server.calls
    assert dumped_of(ours) == dumped_of(reference)
    assert query_of(call) == {}
    assert query_of(reference_call) == {}
    assert query_of(reference_call) == query_of(call)


def test_calls_retrieve() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        reference = REFERENCE.retrieve_container(
            container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY
        )
        ours = retrieve_container(container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY)
        async_reference = asyncio.run(
            REFERENCE.aretrieve_container(
                container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY
            )
        )
        async_ours = asyncio.run(
            aretrieve_container(container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY)
        )
        recorded = list(server.calls)
    assert dumped_of(ours) == dumped_of(reference)
    assert dumped_of(async_ours) == dumped_of(async_reference)
    assert [call["path"] for call in recorded] == [f"{CONTAINERS_PATH}/{CONTAINER_ID}"] * 4
    assert hidden_of(ours) == {}
    assert hidden_of(async_ours) == {}


def test_calls_delete() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        answers = [
            delete_container(container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY),
            REFERENCE.delete_container(
                container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY
            ),
            asyncio.run(
                adelete_container(container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY)
            ),
            asyncio.run(
                REFERENCE.adelete_container(
                    container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY
                )
            ),
        ]
        recorded = list(server.calls)
    assert [answer.deleted for answer in answers] == [True] * 4
    assert [answer.id for answer in answers] == [CONTAINER_ID] * 4
    assert [dumped_of(answer) for answer in answers] == [dumped_of(answers[0])] * 4
    assert {call["method"] for call in recorded} == {"DELETE"}


def test_calls_extra_params() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        sent_headers = {PROBE_HEADER: PROBE_VALUE}
        sent_query = {PAGE_PARAM: PAGE_VALUE}
        created = create_container(
            name=CONTAINER_NAME, api_base=base, api_key=GOOD_KEY, extra_headers=sent_headers
        )
        reference_created = REFERENCE.create_container(
            name=CONTAINER_NAME, api_base=base, api_key=GOOD_KEY, extra_headers=sent_headers
        )
        listed = list_containers(
            api_base=base, api_key=GOOD_KEY, extra_headers=sent_headers, extra_query=sent_query
        )
        reference_listed = REFERENCE.list_containers(
            api_base=base, api_key=GOOD_KEY, extra_headers=sent_headers, extra_query=sent_query
        )
        got = retrieve_container(
            container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY, extra_query=sent_query
        )
        reference_got = REFERENCE.retrieve_container(
            container_id=CONTAINER_ID, api_base=base, api_key=GOOD_KEY, extra_query=sent_query
        )
        recorded = list(server.calls)
    [create_call, reference_create_call] = recorded[:2]
    [list_call, reference_list_call] = recorded[2:4]
    [get_call, reference_get_call] = recorded[4:]
    assert dumped_of(created) == dumped_of(reference_created)
    assert dumped_of(listed) == dumped_of(reference_listed)
    assert dumped_of(got) == dumped_of(reference_got)
    # create keeps extra_headers out of the body and sends them as a header, as LiteLLM does
    assert body_of(create_call) == body_of(reference_create_call)
    assert "extra_headers" not in body_of(create_call)
    assert header_of(create_call, PROBE_HEADER) == header_of(reference_create_call, PROBE_HEADER)
    assert header_of(create_call, PROBE_HEADER) == PROBE_VALUE
    # list and retrieve send extra_query as a query parameter and the header unchanged
    assert query_of(list_call) == query_of(reference_list_call)
    assert query_of(list_call) == {PAGE_PARAM: PAGE_VALUE}
    assert header_of(list_call, PROBE_HEADER) == header_of(reference_list_call, PROBE_HEADER)
    assert header_of(list_call, PROBE_HEADER) == PROBE_VALUE
    assert query_of(get_call) == query_of(reference_get_call)
    assert query_of(get_call) == {PAGE_PARAM: PAGE_VALUE}


def test_error_retrieve_missing() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        with pytest.raises(NotFoundError) as caught:
            retrieve_container(container_id=MISSING_ID, api_base=base, api_key=GOOD_KEY)
        with pytest.raises(Exception, match=NOT_FOUND_TEXT) as reference_caught:
            REFERENCE.retrieve_container(container_id=MISSING_ID, api_base=base, api_key=GOOD_KEY)
    error = caught.value
    assert error.status_code == NOT_FOUND
    assert error.__class__.__name__ == reference_caught.value.__class__.__name__
    assert NOT_FOUND_TEXT in str(error)
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeContainerError"


def test_error_delete_async() -> None:
    with closing(ContainerServer()) as server:
        base = f"{server.url}{API_BASE_PATH}"
        with pytest.raises(NotFoundError) as caught:
            asyncio.run(
                adelete_container(container_id=MISSING_ID, api_base=base, api_key=GOOD_KEY)
            )
        with pytest.raises(Exception, match=NOT_FOUND_TEXT):
            asyncio.run(
                REFERENCE.adelete_container(
                    container_id=MISSING_ID, api_base=base, api_key=GOOD_KEY
                )
            )
    error = caught.value
    assert error.status_code == NOT_FOUND
    assert NOT_FOUND_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeContainerError"


def test_unsupported_arguments() -> None:
    with pytest.raises(UnsupportedCallArgument) as client_caught:
        create_container(name=CONTAINER_NAME, client=object())
    assert client_caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_caught:
        list_containers(shared_session=object())
    assert session_caught.value.argument == "shared_session"
    with pytest.raises(UnsupportedCallArgument) as token_caught:
        retrieve_container(container_id=CONTAINER_ID, azure_ad_token_provider=object())
    assert token_caught.value.argument == "azure_ad_token_provider"


def test_unsupported_async() -> None:
    with pytest.raises(UnsupportedCallArgument):
        asyncio.run(acreate_container(name=CONTAINER_NAME, client=object()))
    with pytest.raises(UnsupportedCallArgument):
        asyncio.run(adelete_container(container_id=CONTAINER_ID, shared_session=object()))
