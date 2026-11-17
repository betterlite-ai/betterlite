import os
import threading
from collections.abc import Awaitable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Literal, NotRequired, TypedDict

import httpx
import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from litellm.utils import get_valid_models
from pydantic import BaseModel, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

PLACEHOLDER: Final = "{{upstream}}"
KEPT_HEADERS: Final[frozenset[str]] = frozenset({"authorization"})


class Raised(BaseModel):
    status_code: int


class Answer(TypedDict):
    status: int
    body: str


class Message(TypedDict):
    role: str
    content: str


class Globals(TypedDict):
    api_key: NotRequired[str]
    api_base: NotRequired[str]
    ollama_key: NotRequired[str]
    lemonade_key: NotRequired[str]
    openai_key: NotRequired[str]
    secret_manager_api_key: NotRequired[str]


class Case(TypedDict):
    op: Literal["chat", "lemonade_models", "lemonade_reply", "ollama_models"]
    provider: Literal[
        "hosted_vllm",
        "ollama",
        "ollama_chat",
        "lemonade",
        "petals",
        "triton",
        "oobabooga",
        "xinference",
    ]
    model: NotRequired[str]
    messages: NotRequired[list[Message]]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    stream: NotRequired[bool]
    env: NotRequired[dict[str, str]]
    globals: NotRequired[Globals]
    catalog: NotRequired[list[str]]
    answer: NotRequired[Answer]


Created = ModelResponse | CustomStreamWrapper | Awaitable[ModelResponse | CustomStreamWrapper]


class Recorder:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.requests: list[tuple[str, dict[str, str]]] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        headers = {
            name: request.headers[name]
            for name in KEPT_HEADERS
            if name in request.headers
        }
        self.requests.append((str(request.url), headers))
        return httpx.Response(
            self.answer["status"],
            content=self.answer["body"].encode(),
            headers={"content-type": "application/json"},
        )


class RecordingServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, answer: Answer) -> None:
        super().__init__(("127.0.0.1", 0), _Handler)
        self.answer = answer
        self.requests: list[tuple[str, dict[str, str]]] = []
        _Handler.recording = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    recording: ClassVar["RecordingServer | None"] = None

    def do_GET(self) -> None:
        server = _Handler.recording
        if server is None:
            raise RuntimeError("the handler is not on a RecordingServer")
        lowered = {name.lower(): value for name, value in self.headers.items()}
        kept = {
            name: lowered[name]
            for name in KEPT_HEADERS
            if name in lowered
        }
        server.requests.append((server.base + self.path, kept))
        payload = server.answer["body"].encode()
        self.send_response(server.answer["status"])
        self.send_header("content-length", str(len(payload)))
        self.send_header("content-type", "application/json")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(payload)


def replace_environment(case: Case, upstream: str) -> None:
    os.environ.clear()
    values = case.get("env")
    if values is None:
        return
    os.environ.update(
        {name: value.replace(PLACEHOLDER, upstream) for name, value in values.items()}
    )


def apply_globals(values: Globals | None) -> None:
    if values is None:
        return
    litellm.api_base = values.get("api_base")
    litellm.api_key = values.get("api_key")
    litellm.ollama_key = values.get("ollama_key")
    litellm.lemonade_key = values.get("lemonade_key")
    litellm.openai_key = values.get("openai_key")


def answered(case: Case) -> Answer:
    reported = case.get("answer")
    if reported is not None:
        return reported
    return {"status": 200, "body": ""}


def substituted_base(case: Case, upstream: str) -> str | None:
    api_base = case.get("api_base")
    if api_base is None:
        return None
    return api_base.replace(PLACEHOLDER, upstream)


def request_view(
    requests: list[tuple[str, dict[str, str]]], upstream: str
) -> JsonValue:
    # LiteLLM answers a model-info lookup for some providers before the request being compared
    for url, recorded in requests:
        if not url.endswith("/api/show") and "/models/" not in url:
            headers: dict[str, JsonValue] = dict[str, JsonValue](recorded.items())
            return {"url": url.replace(upstream, PLACEHOLDER), "headers": headers}
    return None


def prepared(case: Case, upstream: str) -> tuple[Recorder, HTTPHandler]:
    recorder = Recorder(answered(case))
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle)))
    litellm.module_level_client = client
    replace_environment(case, upstream)
    apply_globals(case.get("globals"))
    return recorder, client


def completion_created(case: Case, client: HTTPHandler, model: str) -> Created:
    created: Created = litellm.completion(
        model=model,
        messages=case.get("messages"),
        api_base=substituted_base(case, PLACEHOLDER),
        api_key=case.get("api_key"),
        headers=case.get("headers"),
        stream=case.get("stream"),
        client=client,
    )
    return created


def named_model(case: Case) -> str:
    model = case.get("model")
    if model is None:
        raise ValueError("the case names no model")
    return model


def chat_value(case: Case) -> JsonValue:
    recorder, client = prepared(case, PLACEHOLDER)
    error: JsonValue = None
    try:
        completion_created(case, client, f"{case['provider']}/{named_model(case)}")
    except openai.APIError as raised:
        error = {"status": Raised.model_validate(raised, from_attributes=True).status_code}
    return {"request": request_view(recorder.requests, PLACEHOLDER), "error": error}


def lemonade_reply_value(case: Case) -> JsonValue:
    _recorder, client = prepared(case, PLACEHOLDER)
    created = completion_created(case, client, f"lemonade/{named_model(case)}")
    reply = TypeAdapter(ModelResponse).validate_python(created)
    return {"model": reply.model}


def lemonade_models_value(case: Case) -> JsonValue:
    recorder, _client = prepared(case, PLACEHOLDER)
    models = get_valid_models(
        check_provider_endpoint=True,
        custom_llm_provider="lemonade",
        api_base=substituted_base(case, PLACEHOLDER),
        api_key=case.get("api_key"),
    )
    return {"models": list(models), "request": request_view(recorder.requests, PLACEHOLDER)}


def ollama_models_value(case: Case, server: RecordingServer) -> JsonValue:
    replace_environment(case, server.base)
    apply_globals(case.get("globals"))
    models = get_valid_models(
        check_provider_endpoint=True,
        custom_llm_provider="ollama",
        api_base=substituted_base(case, server.base),
        api_key=case.get("api_key"),
    )
    return {"models": list(models), "request": request_view(server.requests, server.base)}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    if case["op"] == "ollama_models":
        server = RecordingServer(answered(case))
        try:
            return ollama_models_value(case, server)
        finally:
            server.close()
    if case["op"] == "chat":
        return chat_value(case)
    if case["op"] == "lemonade_reply":
        return lemonade_reply_value(case)
    return lemonade_models_value(case)
