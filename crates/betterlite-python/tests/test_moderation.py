import asyncio
import importlib
import inspect
import json
import threading
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite.exceptions import AuthenticationError
from betterlite.main import amoderation, moderation

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
DEFAULT_MODEL: Final = "omni-moderation-latest"
EXPLICIT_MODEL: Final = "text-moderation-latest"
PROVIDER: Final = "openai"
MODERATIONS_PATH: Final = "/v1/moderations"
API_BASE_PATH: Final = "/v1"
JSON_TYPE: Final = "application/json"
MODERATION_ID: Final = "modr-1"
INPUT_TEXT: Final = "hello"
CATEGORY_COUNT: Final = 13
# the provider keys with a slash or hyphen that the client body repeats beside its field names
ALIAS_COUNT: Final = 8
HATE_SCORE: Final = 0.01
AUTH_TEXT: Final = "Incorrect API key provided: bad-key"
MISSING_KEY_TEXT: Final = "no API key is set"
CATEGORIES: Final[dict[str, JsonValue]] = {
    "harassment": False,
    "harassment/threatening": False,
    "hate": False,
    "hate/threatening": False,
    "illicit": False,
    "illicit/violent": False,
    "self-harm": False,
    "self-harm/instructions": False,
    "self-harm/intent": False,
    "sexual": False,
    "sexual/minors": False,
    "violence": False,
    "violence/graphic": False,
}
APPLIED: Final[dict[str, JsonValue]] = {
    name: ["text"] for name in CATEGORIES
}
SCORES: Final[dict[str, JsonValue]] = {
    "harassment": HATE_SCORE,
    "harassment/threatening": HATE_SCORE,
    "hate": HATE_SCORE,
    "hate/threatening": HATE_SCORE,
    "illicit": HATE_SCORE,
    "illicit/violent": HATE_SCORE,
    "self-harm": HATE_SCORE,
    "self-harm/instructions": HATE_SCORE,
    "self-harm/intent": HATE_SCORE,
    "sexual": HATE_SCORE,
    "sexual/minors": HATE_SCORE,
    "violence": HATE_SCORE,
    "violence/graphic": HATE_SCORE,
}
AUTH_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": AUTH_TEXT,
        "type": "invalid_request_error",
        "param": None,
        "code": "invalid_api_key",
    }
}

BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
MODEL: Final[TypeAdapter[str]] = TypeAdapter(str)


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: dict[str, JsonValue]


class ResultView(TypedDict):
    flagged: bool
    categories: dict[str, JsonValue]
    category_scores: dict[str, JsonValue]
    category_applied_input_types: dict[str, JsonValue]


class ModerationView(TypedDict):
    id: str
    model: str
    results: list[ResultView]


VIEW: Final[TypeAdapter[ModerationView]] = TypeAdapter(ModerationView)


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


REFERENCE: ReferenceModule = importlib.import_module("litellm")


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def moderation_body(model: str) -> dict[str, JsonValue]:
    result: dict[str, JsonValue] = {
        "flagged": False,
        "categories": CATEGORIES,
        "category_scores": SCORES,
        "category_applied_input_types": APPLIED,
    }
    return {"id": MODERATION_ID, "model": model, "results": [result]}


def answered_model(body: dict[str, JsonValue]) -> str:
    given = body.get("model")
    return DEFAULT_MODEL if given is None else MODEL.validate_python(given)


class ModerationServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), ModerationHandler)
        self.calls = []
        ModerationHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}{API_BASE_PATH}"

    def record(self, path: str, headers: dict[str, str], body: dict[str, JsonValue]) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class ModerationHandler(BaseHTTPRequestHandler):
    owner: ClassVar[ModerationServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = BODY.validate_json(raw)
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, AUTH_BODY)
            return
        self.reply(OK, moderation_body(answered_model(body)))

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def header_of(call: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call["headers"].items():
        if header.lower() == lowered:
            return value
    return None


def first_result(view: ModerationView) -> ResultView:
    results = view.get("results")
    assert results is not None
    return results[0]


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(moderation)) == parameter_rows(
        inspect.signature(REFERENCE.moderation)
    )
    assert parameter_rows(inspect.signature(amoderation)) == parameter_rows(
        inspect.signature(REFERENCE.amoderation)
    )
    assert not inspect.iscoroutinefunction(moderation)
    assert inspect.iscoroutinefunction(amoderation)


def asked(model: str | None = None) -> tuple[ModerationView, RecordedCall]:
    with closing(ModerationServer()) as server:
        response = moderation(input=INPUT_TEXT, model=model, api_base=server.url, api_key=GOOD_KEY)
        call = server.calls[-1]
    return VIEW.validate_json(response.model_dump_json()), call


async def asked_async(model: str | None = None) -> tuple[ModerationView, RecordedCall]:
    with closing(ModerationServer()) as server:
        response = await amoderation(
            input=INPUT_TEXT, model=model, api_base=server.url, api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    return VIEW.validate_json(response.model_dump_json()), call


# betterlite's wire names omni-moderation-latest where LiteLLM's create(input=input) omits it
def test_default_model_on_wire() -> None:
    view, call = asked()
    assert call["path"] == MODERATIONS_PATH
    assert call["body"]["model"] == DEFAULT_MODEL
    assert call["body"]["input"] == INPUT_TEXT
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert view["id"] == MODERATION_ID
    assert view["model"] == DEFAULT_MODEL


def test_results_carry_categories() -> None:
    result = first_result(asked()[0])
    categories = result["categories"]
    assert result["flagged"] is False
    assert len(categories) == CATEGORY_COUNT + ALIAS_COUNT
    assert categories["hate"] is False
    assert categories["self_harm"] is False
    assert categories["self-harm"] is False
    assert result["category_scores"]["violence"] == HATE_SCORE
    assert result["category_scores"]["violence/graphic"] == HATE_SCORE
    assert result["category_applied_input_types"]["hate"] == ["text"]
    assert result["category_applied_input_types"]["self-harm"] == ["text"]


def test_explicit_model_is_sent() -> None:
    view, call = asked(EXPLICIT_MODEL)
    assert call["body"]["model"] == EXPLICIT_MODEL
    assert view["model"] == EXPLICIT_MODEL


def test_bad_key_auth_error() -> None:
    server = ModerationServer()
    try:
        with pytest.raises(AuthenticationError) as caught:
            moderation(input=INPUT_TEXT, api_base=server.url, api_key=BAD_KEY)
    finally:
        server.close()
    error = caught.value
    assert error.status_code == UNAUTHORIZED
    assert error.model == DEFAULT_MODEL
    assert error.llm_provider == PROVIDER
    assert AUTH_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeTextError"


def test_missing_key_is_plain(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    with closing(ModerationServer()) as server, pytest.raises(
        Exception, match=MISSING_KEY_TEXT
    ) as caught:
        moderation(input=INPUT_TEXT, api_base=server.url)
    assert caught.value.__class__ is Exception
    assert server.calls == []


def test_async_returns_coroutine() -> None:
    with closing(ModerationServer()) as server:
        pending = amoderation(input=INPUT_TEXT, api_base=server.url, api_key=GOOD_KEY)
        assert inspect.isawaitable(pending)
        response = asyncio.run(pending)
    assert VIEW.validate_json(response.model_dump_json())["id"] == MODERATION_ID


@pytest.mark.parametrize(
    ("given", "sent"),
    [(None, DEFAULT_MODEL), (f"{PROVIDER}/{EXPLICIT_MODEL}", EXPLICIT_MODEL)],
    ids=["default", "provider_prefix"],
)
def test_async_model_on_wire(given: str | None, sent: str) -> None:
    view, call = asyncio.run(asked_async(given))
    assert call["body"]["model"] == sent
    assert view["model"] == sent
