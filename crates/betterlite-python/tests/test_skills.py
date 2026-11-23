import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Coroutine
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Protocol, Self, TypedDict

import pytest
from pydantic import BaseModel, JsonValue
from typing_extensions import override

import betterlite.exceptions as our_errors
from betterlite import skills
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import CallValue
from betterlite.exceptions import BadRequestError, NotFoundError
from betterlite.skills import (
    acreate_skill,
    adelete_skill,
    aget_skill,
    alist_skills,
    create_skill,
    delete_skill,
    get_skill,
    list_skills,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
BAD_REQUEST: Final = 400
NOT_FOUND: Final = 404
SERVER_ERROR: Final = 500
GOOD_KEY: Final = "sk-test"
SKILL_ID: Final = "skill_1"
SKILLS_PATH: Final = "/v1/skills"
TITLE: Final = "Test skill"
CUSTOM_HEADER: Final = "x-skill-test"
CUSTOM_VALUE: Final = "on"
TIMEOUT: Final = 30.0
ANTHROPIC: Final = "anthropic"
OPENAI: Final = "openai"
HOSTED: Final = "litellm_proxy"
NO_DATABASE: Final = "Prisma client is not initialized"
ANTHROPIC_VERSION: Final = "2023-06-01"
SKILLS_BETA: Final = "skills-2025-10-02"
JSON_TYPE: Final = "application/json"
FILE_BYTES: Final = b"# skill"
NAME_MARKER: Final = b'; name="'
FILE_MARKER: Final = b'; filename="'
QUERY: Final[dict[str, JsonValue]] = {"limit": 7}
EMPTY: Final[dict[str, JsonValue]] = {}
SKILL_BODY: Final[dict[str, JsonValue]] = {
    "id": SKILL_ID,
    "created_at": "2025-01-01T00:00:00Z",
    "display_title": TITLE,
    "latest_version": "v1",
    "source": "custom",
    "type": "skill",
    "updated_at": "2025-01-02T00:00:00Z",
    "description": None,
    "search_score": None,
}
LIST_BODY: Final[dict[str, JsonValue]] = {
    "data": [SKILL_BODY],
    "next_page": None,
    "has_more": False,
}
DELETED_BODY: Final[dict[str, JsonValue]] = {"id": SKILL_ID, "type": "skill_deleted"}
BAD_BODY: Final[dict[str, JsonValue]] = {
    "type": "error",
    "error": {"type": "invalid_request_error", "message": "display_title is invalid"},
}
MISSING_BODY: Final[dict[str, JsonValue]] = {
    "type": "error",
    "error": {"type": "not_found_error", "message": "skill not found"},
}


class Recorded(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class RefCall(Protocol):
    def __call__(self, **values: CallValue) -> BaseModel: ...


class RefAsyncCall(Protocol):
    def __call__(self, **values: CallValue) -> Coroutine[None, None, BaseModel]: ...


class RefModule(Protocol):
    def __getattr__(self, name: str) -> RefCall: ...


class RefAsyncModule(Protocol):
    def __getattr__(self, name: str) -> RefAsyncCall: ...


class SkillCall(Protocol):
    def __call__(self, skill_id: str, extra_headers: dict[str, str] | None = None,
                 extra_query: dict[str, JsonValue] | None = None, timeout: float | None = None,
                 custom_llm_provider: str | None = None,
                 **kwargs: CallValue) -> BaseModel: ...


class AsyncSkillCall(Protocol):
    def __call__(self, skill_id: str, extra_headers: dict[str, str] | None = None,
                 extra_query: dict[str, JsonValue] | None = None, timeout: float | None = None,
                 custom_llm_provider: str | None = None,
                 **kwargs: CallValue) -> Coroutine[None, None, BaseModel]: ...


class SkillsReferenceError(Exception):
    status_code: int


class RefErrorModule(Protocol):
    def __getattr__(self, name: str) -> type[SkillsReferenceError]: ...


REFERENCE: RefModule = importlib.import_module("litellm")
REFERENCE_ASYNC: RefAsyncModule = importlib.import_module("litellm")
REFERENCE_ERRORS: RefErrorModule = importlib.import_module("litellm.exceptions")


class SkillsServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[Recorded]
    failing: bool

    def __init__(self, *, failing: bool = False) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        self.failing = failing
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def record(self, method: str, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"method": method, "path": path, "headers": headers, "body": body})

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
    owner: ClassVar[SkillsServer]

    def do_POST(self) -> None:
        self.owner.record("POST", self.path, self.headers_dict(), self.body_bytes())
        if self.owner.failing:
            self.reply(BAD_REQUEST, BAD_BODY)
            return
        self.reply(OK, SKILL_BODY)

    def do_GET(self) -> None:
        self.owner.record("GET", self.path, self.headers_dict(), self.body_bytes())
        if self.owner.failing:
            self.reply(NOT_FOUND, MISSING_BODY)
            return
        if self.path.partition("?")[0] == SKILLS_PATH:
            self.reply(OK, LIST_BODY)
            return
        self.reply(OK, SKILL_BODY)

    def do_DELETE(self) -> None:
        self.owner.record("DELETE", self.path, self.headers_dict(), self.body_bytes())
        if self.owner.failing:
            self.reply(NOT_FOUND, MISSING_BODY)
            return
        self.reply(OK, DELETED_BODY)

    def headers_dict(self) -> dict[str, str]:
        return dict(self.headers.items())

    def body_bytes(self) -> bytes:
        declared = self.headers.get("Content-Length")
        if declared is None:
            return b""
        return self.rfile.read(int(declared))

    def reply(self, status: int, body: dict[str, JsonValue]) -> None:
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", JSON_TYPE)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def dumped(model: BaseModel) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = model.model_dump(mode="json")
    return values


def header_of(call: Recorded, name: str) -> str | None:
    lowered = {key.lower(): value for key, value in call["headers"].items()}
    return lowered.get(name.lower())


def part_field(body: bytes, marker: bytes) -> list[str]:
    chunks = body.split(marker)[1:]
    return [chunk.partition(b'"')[0].decode() for chunk in chunks]


def test_signature_names() -> None:
    assert skills.__all__ == [
        "acreate_skill",
        "adelete_skill",
        "aget_skill",
        "alist_skills",
        "create_skill",
        "delete_skill",
        "get_skill",
        "list_skills",
    ]
    assert skills.create_skill is create_skill
    assert skills.acreate_skill is acreate_skill
    assert not inspect.iscoroutinefunction(create_skill)
    assert not inspect.iscoroutinefunction(get_skill)
    assert inspect.iscoroutinefunction(acreate_skill)
    assert inspect.iscoroutinefunction(adelete_skill)


def test_signature_create() -> None:
    with SkillsServer() as server:
        ours = create_skill(
            files=[("SKILL.md", FILE_BYTES, "text/markdown")],
            display_title=TITLE,
            extra_headers={CUSTOM_HEADER: CUSTOM_VALUE},
            extra_query=QUERY,
            extra_body=EMPTY,
            timeout=TIMEOUT,
            custom_llm_provider=ANTHROPIC,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = REFERENCE.create_skill(
            files=[("SKILL.md", FILE_BYTES, "text/markdown")],
            display_title=TITLE,
            extra_headers={CUSTOM_HEADER: CUSTOM_VALUE},
            extra_query=QUERY,
            extra_body=EMPTY,
            timeout=TIMEOUT,
            custom_llm_provider=ANTHROPIC,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        awaited = asyncio.run(
            acreate_skill(
                display_title=TITLE,
                timeout=TIMEOUT,
                custom_llm_provider=ANTHROPIC,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        our_call = server.calls[-3]
    assert dumped(ours) == dumped(reference) == dumped(awaited)
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE
    assert part_field(our_call["body"], FILE_MARKER) == ["SKILL.md"]


def test_signature_list() -> None:
    with SkillsServer() as server:
        ours = list_skills(
            limit=5,
            page="p1",
            source="custom",
            extra_headers={CUSTOM_HEADER: CUSTOM_VALUE},
            extra_query=QUERY,
            timeout=TIMEOUT,
            custom_llm_provider=ANTHROPIC,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = REFERENCE.list_skills(
            limit=5,
            page="p1",
            source="custom",
            extra_headers={CUSTOM_HEADER: CUSTOM_VALUE},
            extra_query=QUERY,
            timeout=TIMEOUT,
            custom_llm_provider=ANTHROPIC,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        awaited = asyncio.run(
            alist_skills(
                limit=5,
                page="p1",
                source="custom",
                timeout=TIMEOUT,
                custom_llm_provider=ANTHROPIC,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        our_call = server.calls[-3]
        their_call = server.calls[-2]
    assert dumped(ours) == dumped(reference) == dumped(awaited)
    expected = f"{SKILLS_PATH}?limit=7&page=p1&source=custom"
    assert our_call["path"] == their_call["path"] == expected
    assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_signature_get_and_delete() -> None:
    cases: tuple[tuple[SkillCall, RefCall, AsyncSkillCall], ...] = (
        (get_skill, REFERENCE.get_skill, aget_skill),
        (delete_skill, REFERENCE.delete_skill, adelete_skill),
    )
    with SkillsServer() as server:
        for ours, theirs, awaited in cases:
            our_answer = ours(
                SKILL_ID,
                extra_headers={CUSTOM_HEADER: CUSTOM_VALUE},
                extra_query=QUERY,
                timeout=TIMEOUT,
                custom_llm_provider=ANTHROPIC,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
            their_answer = theirs(
                skill_id=SKILL_ID,
                extra_headers={CUSTOM_HEADER: CUSTOM_VALUE},
                extra_query=QUERY,
                timeout=TIMEOUT,
                custom_llm_provider=ANTHROPIC,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
            awaited_answer = asyncio.run(
                awaited(
                    SKILL_ID,
                    timeout=TIMEOUT,
                    custom_llm_provider=ANTHROPIC,
                    api_base=server.url,
                    api_key=GOOD_KEY,
                )
            )
            our_call = server.calls[-3]
            assert dumped(our_answer) == dumped(their_answer) == dumped(awaited_answer)
            assert our_call["path"] == f"{SKILLS_PATH}/{SKILL_ID}"
            assert header_of(our_call, CUSTOM_HEADER) == CUSTOM_VALUE


def test_signature_unknown_keyword() -> None:
    with SkillsServer() as server:
        ours = create_skill(display_title=TITLE, api_base=server.url, api_key=GOOD_KEY, unknown=1)
        reference = REFERENCE.create_skill(
            display_title=TITLE, api_base=server.url, api_key=GOOD_KEY, unknown=1
        )
    assert dumped(ours) == dumped(reference)


def test_calls_create() -> None:
    with SkillsServer() as server:
        ours = create_skill(
            files=[("SKILL.md", FILE_BYTES, "text/markdown")],
            display_title=TITLE,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        reference = REFERENCE.create_skill(
            files=[("SKILL.md", FILE_BYTES, "text/markdown")],
            display_title=TITLE,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        our_call = server.calls[-2]
        their_call = server.calls[-1]
        our_type = header_of(our_call, "content-type")
        their_type = header_of(their_call, "content-type")
    assert dumped(ours) == dumped(reference)
    assert our_call["path"] == their_call["path"] == SKILLS_PATH
    assert part_field(our_call["body"], NAME_MARKER) == ["display_title", "files[]"]
    assert part_field(their_call["body"], NAME_MARKER) == ["display_title", "files[]"]
    assert part_field(our_call["body"], FILE_MARKER) == ["SKILL.md"]
    assert FILE_BYTES in our_call["body"]
    assert FILE_BYTES in their_call["body"]
    assert our_type is not None
    assert their_type is not None
    assert our_type.partition(";")[0] == their_type.partition(";")[0] == "multipart/form-data"
    assert header_of(our_call, "x-api-key") == header_of(their_call, "x-api-key") == GOOD_KEY
    assert header_of(our_call, "anthropic-beta") == SKILLS_BETA
    assert header_of(their_call, "anthropic-beta") == SKILLS_BETA
    assert header_of(our_call, "anthropic-version") == ANTHROPIC_VERSION
    assert header_of(their_call, "anthropic-version") == ANTHROPIC_VERSION


def test_calls_create_json() -> None:
    with SkillsServer() as server:
        ours = create_skill(display_title=TITLE, api_base=server.url, api_key=GOOD_KEY)
        reference = REFERENCE.create_skill(
            display_title=TITLE, api_base=server.url, api_key=GOOD_KEY
        )
        our_call = server.calls[-2]
        their_call = server.calls[-1]
        our_body: dict[str, JsonValue] = json.loads(our_call["body"])
        their_body: dict[str, JsonValue] = json.loads(their_call["body"])
    assert dumped(ours) == dumped(reference)
    assert our_body == their_body == {"display_title": TITLE}
    assert header_of(our_call, "content-type") == JSON_TYPE
    assert header_of(their_call, "content-type") == JSON_TYPE


def test_calls_list() -> None:
    with SkillsServer() as server:
        ours = list_skills(
            limit=5, page="p1", source="custom", api_base=server.url, api_key=GOOD_KEY
        )
        reference = REFERENCE.list_skills(
            limit=5, page="p1", source="custom", api_base=server.url, api_key=GOOD_KEY
        )
        our_call = server.calls[-2]
        their_call = server.calls[-1]
    assert dumped(ours) == dumped(reference)
    expected = f"{SKILLS_PATH}?limit=5&page=p1&source=custom"
    assert our_call["path"] == their_call["path"] == expected


def test_calls_get_and_delete() -> None:
    cases: tuple[tuple[SkillCall, RefCall, str], ...] = (
        (get_skill, REFERENCE.get_skill, "GET"),
        (delete_skill, REFERENCE.delete_skill, "DELETE"),
    )
    with SkillsServer() as server:
        for ours, theirs, method in cases:
            our_answer = ours(SKILL_ID, api_base=server.url, api_key=GOOD_KEY)
            their_answer = theirs(skill_id=SKILL_ID, api_base=server.url, api_key=GOOD_KEY)
            our_call = server.calls[-2]
            their_call = server.calls[-1]
            assert dumped(our_answer) == dumped(their_answer)
            assert our_call["path"] == their_call["path"] == f"{SKILLS_PATH}/{SKILL_ID}"
            assert our_call["method"] == their_call["method"] == method


def test_calls_bytes_file() -> None:
    with SkillsServer() as server:
        ours = create_skill(files=[FILE_BYTES], api_base=server.url, api_key=GOOD_KEY)
        reference = REFERENCE.create_skill(
            files=[FILE_BYTES], api_base=server.url, api_key=GOOD_KEY
        )
        our_call = server.calls[-2]
        their_call = server.calls[-1]
    assert dumped(ours) == dumped(reference)
    assert part_field(our_call["body"], FILE_MARKER) == ["file"]
    assert part_field(their_call["body"], FILE_MARKER) == ["upload"]
    assert FILE_BYTES in our_call["body"]
    assert FILE_BYTES in their_call["body"]


def test_calls_async() -> None:
    with SkillsServer() as server:
        created = asyncio.run(
            acreate_skill(display_title=TITLE, api_base=server.url, api_key=GOOD_KEY)
        )
        listed = asyncio.run(alist_skills(api_base=server.url, api_key=GOOD_KEY))
        fetched = asyncio.run(aget_skill(SKILL_ID, api_base=server.url, api_key=GOOD_KEY))
        deleted = asyncio.run(adelete_skill(SKILL_ID, api_base=server.url, api_key=GOOD_KEY))
        reference_created = asyncio.run(
            REFERENCE_ASYNC.acreate_skill(
                display_title=TITLE, api_base=server.url, api_key=GOOD_KEY
            )
        )
        reference_listed = asyncio.run(
            REFERENCE_ASYNC.alist_skills(api_base=server.url, api_key=GOOD_KEY)
        )
        reference_fetched = asyncio.run(
            REFERENCE_ASYNC.aget_skill(
                skill_id=SKILL_ID, api_base=server.url, api_key=GOOD_KEY
            )
        )
        reference_deleted = asyncio.run(
            REFERENCE_ASYNC.adelete_skill(
                skill_id=SKILL_ID, api_base=server.url, api_key=GOOD_KEY
            )
        )
    assert dumped(created) == dumped(reference_created)
    assert dumped(listed) == dumped(reference_listed)
    assert dumped(fetched) == dumped(reference_fetched)
    assert dumped(deleted) == dumped(reference_deleted)


def test_error_create() -> None:
    with SkillsServer(failing=True) as server:
        with pytest.raises(BadRequestError) as ours:
            create_skill(display_title=TITLE, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(REFERENCE_ERRORS.BadRequestError) as theirs:
            REFERENCE.create_skill(
                display_title=TITLE, api_base=server.url, api_key=GOOD_KEY
            )
    assert ours.value.status_code == theirs.value.status_code == BAD_REQUEST
    assert "display_title is invalid" in str(ours.value)
    assert "display_title is invalid" in str(theirs.value)
    assert "AnthropicException" in str(ours.value)
    cause = ours.value.__cause__
    assert cause is not None
    assert '"BadRequestError"' in str(cause)


def test_error_delete() -> None:
    with SkillsServer(failing=True) as server:
        with pytest.raises(NotFoundError) as ours:
            delete_skill(SKILL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(REFERENCE_ERRORS.NotFoundError) as theirs:
            REFERENCE.delete_skill(skill_id=SKILL_ID, api_base=server.url, api_key=GOOD_KEY)
    assert ours.value.status_code == theirs.value.status_code == NOT_FOUND
    assert "skill not found" in str(ours.value)
    assert "skill not found" in str(theirs.value)


def test_error_get_and_list() -> None:
    with SkillsServer(failing=True) as server:
        with pytest.raises(NotFoundError) as fetched:
            get_skill(SKILL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(REFERENCE_ERRORS.APIConnectionError):
            REFERENCE.get_skill(skill_id=SKILL_ID, api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(NotFoundError) as listed:
            list_skills(api_base=server.url, api_key=GOOD_KEY)
        with pytest.raises(REFERENCE_ERRORS.APIConnectionError):
            REFERENCE.list_skills(api_base=server.url, api_key=GOOD_KEY)
    for error in (fetched, listed):
        assert error.value.status_code == NOT_FOUND
        assert "skill not found" in str(error.value)


def test_error_no_database() -> None:
    with pytest.raises(our_errors.APIConnectionError) as created:
        create_skill(api_key=GOOD_KEY, custom_llm_provider=HOSTED)
    with pytest.raises(our_errors.APIConnectionError) as listed:
        list_skills(api_key=GOOD_KEY, custom_llm_provider=HOSTED)
    with pytest.raises(our_errors.APIConnectionError) as fetched:
        get_skill(SKILL_ID, api_key=GOOD_KEY, custom_llm_provider=HOSTED)
    with pytest.raises(our_errors.APIConnectionError) as deleted:
        delete_skill(SKILL_ID, api_key=GOOD_KEY, custom_llm_provider=HOSTED)
    assert NO_DATABASE in str(created.value)
    assert NO_DATABASE in str(listed.value)
    assert NO_DATABASE in str(fetched.value)
    assert NO_DATABASE in str(deleted.value)


def test_unsupported_provider() -> None:
    with pytest.raises(our_errors.APIConnectionError) as ours:
        create_skill(api_key=GOOD_KEY, custom_llm_provider=OPENAI)
    with pytest.raises(REFERENCE_ERRORS.APIConnectionError) as theirs:
        REFERENCE.create_skill(api_key=GOOD_KEY, custom_llm_provider=OPENAI)
    assert ours.value.status_code == theirs.value.status_code == SERVER_ERROR
    assert "CREATE skill is not supported for openai" in str(ours.value)
    assert "CREATE skill is not supported for openai" in str(theirs.value)


def test_unsupported_object() -> None:
    with pytest.raises(UnsupportedCallArgument) as client_caught:
        create_skill(client=object())
    assert client_caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_caught:
        asyncio.run(aget_skill(SKILL_ID, shared_session=object()))
    assert session_caught.value.argument == "shared_session"
