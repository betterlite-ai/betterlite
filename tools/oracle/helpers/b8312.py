import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, NotRequired, Protocol, TypedDict

import litellm
import openai
from litellm.llms.minimax.text_to_speech.transformation import MinimaxTextToSpeechConfig
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

UPSTREAM: Final = "{{upstream}}"
KEPT: Final[tuple[str, ...]] = ("authorization", "content-type")
BRANDS: Final = ("ARK", "DASHSCOPE", "GIGACHAT", "MINIMAX", "NLP_CLOUD", "QWEN", "VOLCENGINE")
OAUTH_BODY: Final = '{"access_token":"token-1","expires_at":9999999999999}'
ENDPOINTS: Final[dict[str, str]] = {
    "minimax_chat": "/chat/completions", "nlp_cloud_chat": "/generation",
    "volcengine_embedding": "/embeddings", "dashscope_embedding": "/embeddings",
}


class SpeechConfig(Protocol):
    def map_openai_params(
        self,
        _model: str,
        _optional_params: dict[str, JsonValue],
        _voice: str | dict[str, JsonValue] | None,
        /,
        *,
        drop_params: bool,
    ) -> tuple[str | None, dict[str, JsonValue]]: ...


class SpeechFactory(Protocol):
    def __call__(self) -> SpeechConfig: ...


SPEECH: Final[SpeechFactory] = MinimaxTextToSpeechConfig


class Answer(TypedDict):
    status: int
    body: str


class Case(TypedDict):
    op: str
    model: str
    input: NotRequired[str]
    voice: NotRequired[str | dict[str, JsonValue]]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    litellm_api_base: NotRequired[str]
    litellm_api_key: NotRequired[str]
    gigachat_key: NotRequired[str]
    nlp_cloud_key: NotRequired[str]
    env: NotRequired[dict[str, str]]
    answer: NotRequired[Answer]


class Record(TypedDict):
    url: str
    headers: dict[str, JsonValue]
    body: str


class Handler(BaseHTTPRequestHandler):
    current: ClassVar["Exchange | None"] = None

    def do_POST(self) -> None:
        exchange = Handler.current
        if exchange is None:
            raise RuntimeError("the handler is not on an Exchange")
        length = self.headers.get("Content-Length")
        content = b"" if length is None else self.rfile.read(int(length))
        lowered = {name.lower(): value for name, value in self.headers.items()}
        headers: dict[str, JsonValue] = {
            name: lowered[name] for name in KEPT if name in lowered
        }
        exchange.records.append(
            {
                "url": exchange.base + self.path,
                "headers": headers,
                "body": "" if self.path.endswith("/oauth") else content.decode(),
            }
        )
        answer = exchange.answer(self.path)
        payload = answer["body"].encode()
        self.send_response(answer["status"])
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class Exchange(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, answer: Answer) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.given = answer
        self.records: list[Record] = []
        Handler.current = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def answer(self, path: str) -> Answer:
        if path.endswith("/oauth"):
            return {"status": 200, "body": OAUTH_BODY}
        return self.given

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


def prepared(case: Case, exchange: Exchange) -> None:
    for name in list(os.environ):
        if any(brand in name for brand in BRANDS):
            del os.environ[name]
    litellm.api_base = litellm.api_key = litellm.gigachat_key = litellm.nlp_cloud_key = None
    environment = case.get("env")
    if environment is not None:
        for name, value in environment.items():
            os.environ[name] = value.replace(UPSTREAM, exchange.base)
    litellm.api_base = case.get("litellm_api_base")
    if litellm.api_base is not None:
        litellm.api_base = litellm.api_base.replace(UPSTREAM, exchange.base)
    litellm.api_key = case.get("litellm_api_key")
    litellm.gigachat_key = case.get("gigachat_key")
    litellm.nlp_cloud_key = case.get("nlp_cloud_key")
    api_base = case.get("api_base")
    if api_base is not None:
        case["api_base"] = api_base.replace(UPSTREAM, exchange.base)


def text_input(case: Case) -> str:
    text = case.get("input")
    if text is None:
        raise ValueError("the case needs an input")
    return text


def called(case: Case) -> None:
    operation = case["op"]
    provider = operation.removesuffix("_chat").removesuffix("_embedding")
    model = f"{provider}/{case['model']}"
    if operation.endswith("_embedding"):
        litellm.embedding(
            model=model,
            input=text_input(case),
            api_base=case.get("api_base"),
            api_key=case.get("api_key"),
        )
        return
    litellm.completion(
        model=model,
        messages=[{"role": "user", "content": "hi"}],
        api_base=case.get("api_base"),
        api_key=case.get("api_key"),
    )


def speech_view(config: SpeechConfig, case: Case) -> JsonValue:
    voice, params = config.map_openai_params(
        case["model"], {}, case.get("voice"), drop_params=False
    )
    answer: dict[str, JsonValue] = {"voice": voice, "params": params}
    return answer


def value(case: Case, exchange: Exchange) -> JsonValue:
    operation = case["op"]
    if operation == "minimax_speech":
        return speech_view(SPEECH(), case)
    try:
        called(case)
    except openai.APIError as raised:
        # a missing key fails before any request, so litellm's added 500 status is not the arm's
        raise RuntimeError(str(raised)) from raised
    if operation in ("gigachat_chat", "gigachat_embedding"):
        for record in exchange.records:
            if record["url"].endswith("/oauth"):
                authorization = TypeAdapter(str).validate_python(record["headers"]["authorization"])
                return {"credentials": authorization.removeprefix("Basic ")}
        return {"credentials": None}
    for record in exchange.records:
        if not record["url"].endswith(ENDPOINTS[operation]):
            continue
        request: JsonValue = {
            "url": record["url"].replace(exchange.base, UPSTREAM),
            "headers": record["headers"],
        }
        if operation in ("nlp_cloud_chat", "minimax_chat"):
            return {"request": request}
        body = record["body"]
        params: JsonValue = None
        if body:
            params = TypeAdapter(dict[str, JsonValue]).validate_json(body)
        return {"request": request, "params": params}
    return {"request": None}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    answer = case.get("answer")
    given: Answer = {"status": 200, "body": "{}"}
    exchange = Exchange(given if answer is None else answer)
    try:
        prepared(case, exchange)
        return value(case, exchange)
    finally:
        exchange.close()
