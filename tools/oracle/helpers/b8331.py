import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Literal, NotRequired, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

PLACEHOLDER: Final = "{{upstream}}"
UPSTREAM: Final = "https://workspace.example.com"
SIGNED: Final = "<signed>"
REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
SIGNATURE_HEADERS: Final[tuple[str, ...]] = (
    "authorization",
    "x-amz-date",
    "x-amz-security-token",
)
RECORDED_HEADERS: dict[str, tuple[str, ...]] = {
    "sap_chat": ("authorization", "content-type", "ai-resource-group", "ai-client-type"),
    "sap_embedding": ("authorization", "content-type", "ai-resource-group", "ai-client-type"),
    "watsonx_chat": ("authorization", "content-type", "accept"),
    "watsonx_text": ("authorization", "content-type", "accept"),
    "watsonx_embedding": ("authorization", "content-type", "accept"),
    "sagemaker_chat": ("authorization", "content-type", "x-amz-date", "x-amz-security-token"),
    "sagemaker_nova": ("authorization", "content-type", "x-amz-date", "x-amz-security-token"),
}


class Message(TypedDict):
    role: str
    content: str


class Reply(TypedDict):
    status: int
    body: str


class Record(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    body: JsonValue


class Case(TypedDict):
    op: Literal[
        "sap_chat",
        "sap_embedding",
        "watsonx_chat",
        "watsonx_text",
        "watsonx_embedding",
        "sagemaker_chat",
        "sagemaker_nova",
    ]
    model: str
    messages: NotRequired[list[Message]]
    input: NotRequired[str | list[str]]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    stream: NotRequired[bool]
    temperature: NotRequired[float]
    max_tokens: NotRequired[int]
    wx_credentials: NotRequired[dict[str, str]]
    env: NotRequired[dict[str, str]]
    upstream: NotRequired[list[Reply]]


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: HTTPHandler) -> None: ...


CLIENT_CACHE: TypeAdapter[ClientCache] = TypeAdapter(
    ClientCache, config=ConfigDict(arbitrary_types_allowed=True)
)


class Scripted:
    def __init__(self, replies: list[Reply], *, streaming: bool) -> None:
        self.replies = list(replies)
        self.streaming = streaming
        self.records: list[Record] = []
        self.base = UPSTREAM
        self.lock = threading.Lock()

    def answer(self) -> Reply:
        with self.lock:
            if not self.replies:
                raise RuntimeError("provider answer sequence exhausted")
            return self.replies.pop(0)

    def keep(self, record: Record) -> None:
        with self.lock:
            self.records.append(record)

    def recorded(self, method: str, url: str, headers: dict[str, str], content: bytes) -> None:
        body: JsonValue = None
        content_type = headers["content-type"]
        if content:
            body = REPLY.validate_json(content) if "json" in content_type else content.decode()
        self.keep({"method": method, "url": url, "headers": headers, "body": body})

    def handle(self, request: httpx.Request) -> httpx.Response:
        lowered = {name.lower(): value for name, value in request.headers.items()}
        reply = self.answer()
        self.recorded(request.method, str(request.url), lowered, request.content)
        content_type = "text/event-stream" if self.streaming else "application/json"
        return httpx.Response(
            reply["status"],
            content=reply["body"].encode(),
            headers={"content-type": content_type},
        )


class ScriptedHandler(BaseHTTPRequestHandler):
    scripted: ClassVar[Scripted | None] = None

    def answered(self) -> None:
        scripted = ScriptedHandler.scripted
        if scripted is None:
            raise RuntimeError("the handler is not on a ScriptedServer")
        length = self.headers.get("Content-Length")
        content = b"" if length is None else self.rfile.read(int(length))
        lowered = {name.lower(): value for name, value in self.headers.items()}
        reply = scripted.answer()
        scripted.recorded(self.command, scripted.base + self.path, lowered, content)
        payload = reply["body"].encode()
        self.send_response(reply["status"])
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:
        self.answered()

    def do_POST(self) -> None:
        self.answered()


class ScriptedServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, scripted: Scripted) -> None:
        super().__init__(("127.0.0.1", 0), ScriptedHandler)
        scripted.base = self.base
        ScriptedHandler.scripted = scripted
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


def replace_environment(case: Case, upstream: str) -> None:
    os.environ.clear()
    values = case.get("env")
    if values is None:
        return
    os.environ.update(
        {name: value.replace(PLACEHOLDER, upstream) for name, value in values.items()}
    )


def call_provider(case: Case, client: HTTPHandler, upstream: str) -> None:
    model = case["model"]
    messages = case.get("messages")
    raw_base = case.get("api_base")
    api_base = None if raw_base is None else raw_base.replace(PLACEHOLDER, upstream)
    api_key = case.get("api_key")
    match case["op"]:
        case "sap_chat":
            litellm.completion(
                model=f"sap/{model}",
                messages=messages,
                api_key=api_key,
                client=client,
            )
        case "sap_embedding":
            litellm.embedding(
                model=f"sap/{model}",
                input=case.get("input"),
                api_key=api_key,
                client=client,
            )
        case "watsonx_chat":
            litellm.completion(
                model=f"watsonx/{model}",
                messages=messages,
                api_base=api_base,
                api_key=api_key,
                stream=case.get("stream"),
                temperature=case.get("temperature"),
                max_tokens=case.get("max_tokens"),
                client=client,
            )
        case "watsonx_embedding":
            litellm.embedding(
                model=f"watsonx/{model}",
                input=case.get("input"),
                api_base=api_base,
                api_key=api_key,
                client=client,
            )
        case "sagemaker_chat" | "sagemaker_nova":
            litellm.completion(model=f"{case['op']}/{model}", messages=messages, client=client)
        case "watsonx_text":
            text_completion(case, client, upstream, model, api_base)


def credentials_view(case: Case, upstream: str) -> dict[str, str] | None:
    credentials = case.get("wx_credentials")
    if credentials is None:
        return None
    return {name: value.replace(PLACEHOLDER, upstream) for name, value in credentials.items()}


def text_completion(
    case: Case, client: HTTPHandler, upstream: str, model: str, api_base: str | None
) -> None:
    credentials = credentials_view(case, upstream)
    messages = case.get("messages")
    api_key = case.get("api_key")
    temperature = case.get("temperature")
    max_tokens = case.get("max_tokens")
    if credentials is None:
        litellm.completion(
            model=f"watsonx_text/{model}",
            messages=messages,
            api_base=api_base,
            api_key=api_key,
            temperature=temperature,
            max_tokens=max_tokens,
            client=client,
        )
        return
    litellm.completion(
        model=f"watsonx_text/{model}",
        messages=messages,
        api_base=api_base,
        api_key=api_key,
        temperature=temperature,
        max_tokens=max_tokens,
        wx_credentials=credentials,
        client=client,
    )


def header_view(record: Record, op: str) -> dict[str, JsonValue]:
    headers = record["headers"]
    view: dict[str, JsonValue] = {
        name: headers[name] for name in RECORDED_HEADERS[op] if name in headers
    }
    if op in ("sagemaker_chat", "sagemaker_nova"):
        view.update({name: SIGNED for name in SIGNATURE_HEADERS if name in headers})
    return view


def request_view(record: Record, op: str, upstream: str) -> JsonValue:
    return {
        "method": record["method"],
        "url": record["url"].replace(upstream, PLACEHOLDER),
        "headers": header_view(record, op),
        "body": record["body"],
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    replies = case.get("upstream")
    scripted = Scripted([] if replies is None else replies, streaming=case.get("stream") is True)
    server = ScriptedServer(scripted) if case["op"] == "sap_embedding" else None
    upstream = scripted.base
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(scripted.handle)))
    litellm.module_level_client = client
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    CLIENT_CACHE.validate_python(holder).set_cache(key="httpx_client", value=client)
    replace_environment(case, upstream)
    try:
        call_provider(case, client, upstream)
    finally:
        if server is not None:
            server.close()
    return {"requests": [request_view(record, case["op"], upstream) for record in scripted.records]}
