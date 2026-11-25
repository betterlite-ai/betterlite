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

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._text import TextCompletionStreamWrapper
from betterlite.exceptions import (
    AuthenticationError,
    BadRequestError,
    Timeout,
    UnsupportedParamsError,
)
from betterlite.main import atext_completion, text_completion
from betterlite.types.utils import TextCompletionResponse

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
TEXT_MODEL: Final = "gpt-3.5-turbo-instruct"
CHAT_MODEL: Final = "gpt-4o"
TEXT_PROVIDER: Final = "text-completion-openai"
API_BASE_PATH: Final = "/v1"
TEXT_PATH: Final = "/v1/completions"
CHAT_PATH: Final = "/v1/chat/completions"
JSON_TYPE: Final = "application/json"
SSE_TYPE: Final = "text/event-stream"
CREATED: Final = 1700000000
COMPLETION_ID: Final = "cmpl-1"
CHAT_ID: Final = "chatcmpl-1"
CHAT_ANSWER: Final = "chat answer"
FINISH_REASON: Final = "stop"
STREAM_PARTS: Final = ("a", "b", "c")
TOKENS_TEXT: Final = "tokens"
MOCK_TEXT: Final = "mocked"
MOCK_DEFAULT: Final = "This is a mock request"
MOCK_TOOL_CALLS: Final[list[dict[str, JsonValue]]] = [
    {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}},
]
UNKNOWN_PATH: Final = "the server received a path no test expects"
AUTH_TEXT: Final = "Incorrect API key provided: bad-key"
PROMPT_TOKENS: Final = 2
COMPLETION_TOKENS: Final = 1
TOTAL_TOKENS: Final = 3
BEST_OF: Final = 2
MAX_TOKENS: Final = 5
TEMPERATURE: Final = 0.2
TOP_P: Final = 0.9
CHAT_TEMPERATURE: Final = 0.1
BAD_REQUEST: Final = 400
TIMEOUT_STATUS: Final = 408
USAGE: Final[dict[str, JsonValue]] = {
    "prompt_tokens": PROMPT_TOKENS,
    "completion_tokens": COMPLETION_TOKENS,
    "total_tokens": TOTAL_TOKENS,
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


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: dict[str, JsonValue]


class ChoiceView(TypedDict):
    index: int
    text: str | None
    finish_reason: str | None


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseView(TypedDict):
    object: str
    model: str | None
    choices: list[ChoiceView]
    usage: UsageView | None


VIEW: Final[TypeAdapter[ResponseView]] = TypeAdapter(ResponseView)


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


def one_text(value: JsonValue) -> str:
    match value:
        case str():
            return value
        case object():
            return ""


def is_token(value: JsonValue) -> bool:
    match value:
        case int():
            return True
        case str() | float() | None | list() | dict():
            return False


def is_token_list(value: JsonValue) -> bool:
    match value:
        case [first, *_] if is_token(first):
            return True
        case bool() | int() | float() | str() | None | list() | dict():
            return False


def list_prompts(items: list[JsonValue]) -> list[str]:
    match items:
        case []:
            return []
        case [first, *_] if is_token(first) or is_token_list(first):
            return [TOKENS_TEXT]
        case [*_]:
            return [one_text(item) for item in items]


# the prompt as the wire carries it: a string, one per prompt, or one token prompt
def prompt_texts(value: JsonValue) -> list[str]:
    match value:
        case str():
            return [value]
        case list():
            return list_prompts(value)
        case bool() | int() | float() | None | dict():
            return []


def text_frame(index: int, text: str, finish_reason: str | None) -> dict[str, JsonValue]:
    choice: dict[str, JsonValue] = {
        "index": index,
        "text": text,
        "finish_reason": finish_reason,
        "logprobs": None,
    }
    return {
        "id": COMPLETION_ID,
        "object": "text_completion",
        "created": CREATED,
        "model": TEXT_MODEL,
        "choices": [choice],
    }


def usage_frame() -> dict[str, JsonValue]:
    return {
        "id": COMPLETION_ID,
        "object": "text_completion",
        "created": CREATED,
        "model": TEXT_MODEL,
        "choices": [],
        "usage": USAGE,
    }


def chat_frame(content: str | None, finish_reason: str | None) -> dict[str, JsonValue]:
    delta: dict[str, JsonValue] = {} if content is None else {"content": content}
    choice: dict[str, JsonValue] = {
        "index": 0,
        "delta": delta,
        "finish_reason": finish_reason,
    }
    return {
        "id": CHAT_ID,
        "object": "chat.completion.chunk",
        "created": CREATED,
        "model": CHAT_MODEL,
        "choices": [choice],
    }


class TextServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), TextHandler)
        self.calls = []
        TextHandler.owner = self
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


class TextHandler(BaseHTTPRequestHandler):
    owner: ClassVar[TextServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = BODY.validate_json(raw)
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply(UNAUTHORIZED, AUTH_BODY)
            return
        if self.path == TEXT_PATH:
            self.text_answer(body)
            return
        if self.path == CHAT_PATH:
            self.chat_answer(body)
            return
        raise AssertionError(UNKNOWN_PATH)

    def text_answer(self, body: dict[str, JsonValue]) -> None:
        prompts = prompt_texts(body.get("prompt"))
        if body.get("stream") is True:
            self.text_stream(prompts)
            return
        choices: list[JsonValue] = [
            {
                "index": index,
                "text": f"done {prompt}",
                "finish_reason": FINISH_REASON,
                "logprobs": None,
            }
            for index, prompt in enumerate(prompts)
        ]
        payload: dict[str, JsonValue] = {
            "id": COMPLETION_ID,
            "object": "text_completion",
            "created": CREATED,
            "model": TEXT_MODEL,
            "choices": choices,
            "usage": USAGE,
        }
        self.reply(OK, payload)

    def chat_answer(self, body: dict[str, JsonValue]) -> None:
        if body.get("stream") is True:
            self.chat_stream()
            return
        message: dict[str, JsonValue] = {"role": "assistant", "content": CHAT_ANSWER}
        choice: dict[str, JsonValue] = {
            "index": 0,
            "message": message,
            "finish_reason": FINISH_REASON,
        }
        payload: dict[str, JsonValue] = {
            "id": CHAT_ID,
            "object": "chat.completion",
            "created": CREATED,
            "model": CHAT_MODEL,
            "choices": [choice],
            "usage": USAGE,
        }
        self.reply(OK, payload)

    def text_stream(self, prompts: list[str]) -> None:
        self.start_stream()
        for index, prompt in enumerate(prompts):
            for part in STREAM_PARTS:
                self.frame(text_frame(index, f"{prompt} {part}", None))
            self.frame(text_frame(index, "", FINISH_REASON))
        self.frame(usage_frame())
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def chat_stream(self) -> None:
        self.start_stream()
        for part in STREAM_PARTS:
            self.frame(chat_frame(part, None))
        self.frame(chat_frame(None, FINISH_REASON))
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def start_stream(self) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", SSE_TYPE)
        self.end_headers()

    def frame(self, payload: dict[str, JsonValue]) -> None:
        self.wfile.write(f"data: {json.dumps(payload)}\n\n".encode())
        self.wfile.flush()

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


def view_of(response: TextCompletionResponse) -> ResponseView:
    dumped = response.model_dump()
    return VIEW.validate_python(dumped)


def answer_of(
    outcome: TextCompletionResponse | TextCompletionStreamWrapper,
) -> TextCompletionResponse:
    match outcome:
        case TextCompletionResponse():
            return outcome
        case TextCompletionStreamWrapper():
            raise AssertionError("a buffered call answered a stream")


def chunk_views(stream: TextCompletionStreamWrapper) -> list[ResponseView]:
    chunks = [chunk.model_dump() for chunk in stream]
    return [VIEW.validate_python(chunk) for chunk in chunks]


def first_choice(view: ResponseView) -> ChoiceView:
    choices = view.get("choices")
    assert choices is not None
    return choices[0]


async def async_texts_of(stream: TextCompletionStreamWrapper) -> list[ResponseView]:
    chunks = [chunk.model_dump() async for chunk in stream]
    return [VIEW.validate_python(chunk) for chunk in chunks]


# one server and one recorded call per request: the answer arrives as the kind the call asked for
def requested(
    prompt: JsonValue,
    model: str = TEXT_MODEL,
    provider: str | None = None,
    *,
    stream: bool | None = None,
    options: dict[str, JsonValue] | None = None,
) -> tuple[list[ResponseView], RecordedCall]:
    with closing(TextServer()) as server:
        outcome = text_completion(
            model=model,
            prompt=prompt,
            custom_llm_provider=provider,
            stream=stream,
            stream_options=options,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    match outcome:
        case TextCompletionResponse():
            assert stream is not True
            return [view_of(outcome)], call
        case TextCompletionStreamWrapper():
            assert stream is True
            return chunk_views(outcome), call


async def async_requested(prompt: str) -> list[ResponseView]:
    with closing(TextServer()) as server:
        outcome = await atext_completion(
            model=TEXT_MODEL,
            prompt=prompt,
            stream=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
    match outcome:
        case TextCompletionResponse():
            raise AssertionError("a streaming call answered a complete response")
        case TextCompletionStreamWrapper():
            return await async_texts_of(outcome)


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(text_completion)) == parameter_rows(
        inspect.signature(REFERENCE.text_completion)
    )
    assert parameter_rows(inspect.signature(atext_completion)) == parameter_rows(
        inspect.signature(REFERENCE.atext_completion)
    )
    assert not inspect.iscoroutinefunction(text_completion)
    assert inspect.iscoroutinefunction(atext_completion)


@pytest.mark.parametrize(
    ("given", "sent", "count"),
    [
        ("hi", "hi", 1),
        (["only"], "only", 1),
        (["first", "second"], ["first", "second"], 2),
        ([1, 2], [1, 2], 1),
        ([[1, 2], [3, 4]], [[1, 2], [3, 4]], 1),
    ],
    ids=["string", "one_string_list", "strings", "tokens", "token_lists"],
)
def test_prompts_reach_the_wire(given: JsonValue, sent: JsonValue, count: int) -> None:
    views, call = requested(given)
    assert call["path"] == TEXT_PATH
    assert call["body"]["prompt"] == sent
    assert call["body"]["model"] == TEXT_MODEL
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert views[0]["object"] == "text_completion"
    assert [len(view["choices"]) for view in views] == [count]


def test_prompts_join_indexes() -> None:
    views, _call = requested(["first", "second"])
    choices = views[0]["choices"]
    assert [choice.get("index") for choice in choices] == [0, 1]
    assert [choice.get("text") for choice in choices] == ["done first", "done second"]


def test_prompts_parameters_reach() -> None:
    with closing(TextServer()) as server:
        outcome = text_completion(
            model=TEXT_MODEL,
            prompt="hi",
            best_of=2,
            echo=True,
            logit_bias={"50256": -100},
            logprobs=1,
            max_tokens=5,
            n=1,
            stop=["\n"],
            suffix="end",
            temperature=0.2,
            top_p=0.9,
            user="reader",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        body = server.calls[-1]["body"]
    assert body["best_of"] == BEST_OF
    assert body["echo"] is True
    assert body["logit_bias"] == {"50256": -100}
    assert body["logprobs"] == 1
    assert body["max_tokens"] == MAX_TOKENS
    assert body["n"] == 1
    assert body["stop"] == ["\n"]
    assert body["suffix"] == "end"
    assert body["temperature"] == TEMPERATURE
    assert body["top_p"] == TOP_P
    assert body["user"] == "reader"
    assert answer_of(outcome).model_dump()["object"] == "text_completion"


def test_prompts_engine_alias() -> None:
    with closing(TextServer()) as server:
        outcome = text_completion(
            engine=TEXT_MODEL, prompt="hi", api_base=server.url, api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert call["body"]["model"] == TEXT_MODEL
    assert first_choice(view_of(answer_of(outcome)))["text"] == "done hi"


# main.py's async entry hands engine to text_completion, which reads it as the model
def test_prompts_async_engine() -> None:
    with closing(TextServer()) as server:
        outcome = asyncio.run(
            atext_completion(
                model=None, engine=TEXT_MODEL, prompt="hi", api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        call = server.calls[-1]
    assert call["path"] == TEXT_PATH
    assert call["body"]["model"] == TEXT_MODEL
    assert first_choice(view_of(answer_of(outcome)))["text"] == "done hi"


def test_prompts_text_endpoint() -> None:
    views, call = requested("hi", CHAT_MODEL, TEXT_PROVIDER)
    assert call["path"] == TEXT_PATH
    assert call["body"]["model"] == CHAT_MODEL
    assert call["body"]["prompt"] == "hi"
    assert views[0]["object"] == "text_completion"


def test_prompts_unmapped_plain() -> None:
    with pytest.raises(Exception, match="Unmapped prompt format") as caught:
        text_completion(model=TEXT_MODEL, prompt=7)
    assert caught.value.__class__ is Exception


def test_prompts_auth_error() -> None:
    server = TextServer()
    try:
        with pytest.raises(AuthenticationError) as caught:
            text_completion(
                model=TEXT_MODEL,
                prompt="hi",
                api_base=server.url,
                api_key=BAD_KEY,
            )
    finally:
        server.close()
    error = caught.value
    assert error.status_code == UNAUTHORIZED
    assert error.model == TEXT_MODEL
    assert error.llm_provider == TEXT_PROVIDER
    assert AUTH_TEXT in error.message
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeTextError"


def test_prompts_unknown_param() -> None:
    with pytest.raises(UnsupportedParamsError) as caught:
        text_completion(model=TEXT_MODEL, prompt="hi", api_type="openai")
    assert caught.value.status_code == BAD_REQUEST
    assert "api_type" in caught.value.message


# betterlite reports a missing model as a 400 where LiteLLM's async entry raises APIConnectionError
def test_prompts_async_model_none() -> None:
    with pytest.raises(BadRequestError) as caught:
        asyncio.run(atext_completion(model=None, prompt="hi"))
    assert caught.value.status_code == BAD_REQUEST
    assert "LLM Provider NOT provided" in caught.value.message


# main.py's completion entry answers a mock before any provider runs, for a text endpoint model too
def test_prompts_mock_answer() -> None:
    with closing(TextServer()) as server:
        outcome = text_completion(
            model=TEXT_MODEL, prompt="hi", mock_response=MOCK_TEXT, api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert server.calls == []
    assert first_choice(view_of(answer_of(outcome)))["text"] == MOCK_TEXT
    with closing(TextServer()) as tool_server:
        tool_outcome = text_completion(
            model=TEXT_MODEL, prompt="hi", mock_tool_calls=MOCK_TOOL_CALLS,
            api_base=tool_server.url, api_key=GOOD_KEY,
        )
        assert tool_server.calls == []
    assert first_choice(view_of(answer_of(tool_outcome)))["text"] == MOCK_DEFAULT


# a mock answers a token prompt on a text provider, where a chat provider refuses it first
def test_prompts_mock_token_prompt() -> None:
    with closing(TextServer()) as server:
        outcome = text_completion(
            model=TEXT_MODEL, prompt=[1, 2], mock_response=MOCK_TEXT, api_base=server.url,
            api_key=GOOD_KEY,
        )
        with pytest.raises(Exception, match="Unmapped prompt format") as caught:
            text_completion(
                model="claude-3-haiku-20240307", custom_llm_provider="anthropic",
                prompt=[1, 2], mock_response=MOCK_TEXT, api_base=server.url, api_key=GOOD_KEY,
            )
        assert server.calls == []
    assert caught.value.__class__ is Exception
    assert first_choice(view_of(answer_of(outcome)))["text"] == MOCK_TEXT


def test_prompts_mock_timeout() -> None:
    with pytest.raises(Timeout) as caught:
        text_completion(model=TEXT_MODEL, prompt="hi", mock_timeout=True, timeout=0)
    assert caught.value.status_code == TIMEOUT_STATUS


def test_prompts_mock_no_timeout() -> None:
    with pytest.raises(BadRequestError) as caught:
        text_completion(model=TEXT_MODEL, prompt="hi", mock_timeout=True)
    assert caught.value.status_code == BAD_REQUEST


def test_chat_one_user_message() -> None:
    views, call = requested("hello", CHAT_MODEL)
    assert call["path"] == CHAT_PATH
    assert call["body"]["messages"] == [{"role": "user", "content": "hello"}]
    assert "prompt" not in call["body"]
    assert views[0]["object"] == "text_completion"
    assert first_choice(views[0]).get("text") == CHAT_ANSWER
    assert views[0]["model"] == CHAT_MODEL


def test_chat_string_messages() -> None:
    views, call = requested(["first", "second"], CHAT_MODEL)
    messages: Final[JsonValue] = [
        {"role": "user", "content": "first"},
        {"role": "user", "content": "second"},
    ]
    assert call["body"]["messages"] == messages
    assert views[0]["object"] == "text_completion"


def test_chat_parameters_sent() -> None:
    with closing(TextServer()) as server:
        _ = text_completion(
            model=CHAT_MODEL,
            prompt="hello",
            max_tokens=5,
            stop=["x"],
            temperature=0.1,
            user="reader",
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        body = server.calls[-1]["body"]
    assert body["max_tokens"] == MAX_TOKENS
    assert body["stop"] == ["x"]
    assert body["temperature"] == CHAT_TEMPERATURE
    assert body["user"] == "reader"


def test_chat_tokens_refused() -> None:
    with pytest.raises(Exception, match="Unmapped prompt format") as caught:
        text_completion(model=CHAT_MODEL, prompt=[1, 2])
    assert caught.value.__class__ is Exception


def test_chat_client_is_refused() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        text_completion(model=CHAT_MODEL, prompt="hi", client=object())
    assert caught.value.argument == "client"


def test_stream_text_chunks() -> None:
    views, call = requested("hi", stream=True)
    assert call["body"]["stream"] is True
    assert [first_choice(view).get("text") for view in views] == ["hi a", "hi b", "hi c", None]
    assert [first_choice(view).get("finish_reason") for view in views] == [
        None, None, None, FINISH_REASON,
    ]
    assert views[0]["object"] == "text_completion"


def test_stream_text_usage_last() -> None:
    views, call = requested("hi", stream=True, options={"include_usage": True})
    assert call["body"]["stream_options"] == {"include_usage": True}
    last = views[-1]["usage"]
    assert last is not None
    assert last["total_tokens"] == TOTAL_TOKENS
    assert last["prompt_tokens"] == PROMPT_TOKENS
    assert views[0]["usage"] is None


def test_stream_without_usage() -> None:
    views, _call = requested("hi", stream=True)
    usage = views[-1]["usage"]
    assert usage is not None
    assert usage["total_tokens"] == 0


def test_stream_async_text_chunks() -> None:
    views = asyncio.run(async_requested("hi"))
    assert [first_choice(view).get("text") for view in views] == ["hi a", "hi b", "hi c", None]


def test_stream_chat_chunks() -> None:
    views, call = requested("hello", CHAT_MODEL, stream=True)
    assert call["path"] == CHAT_PATH
    texts = [one_text(first_choice(view)["text"]) for view in views]
    assert [text for text in texts if text] == list(STREAM_PARTS)
    assert first_choice(views[-1])["finish_reason"] == FINISH_REASON


def test_stream_unknown_argument() -> None:
    with closing(TextServer()) as server, pytest.raises(UnsupportedCallArgument):
        _ = text_completion(
            model=TEXT_MODEL,
            prompt="hi",
            stream=True,
            shared_session=object(),
            api_base=server.url,
            api_key=GOOD_KEY,
        )
    assert server.calls == []
