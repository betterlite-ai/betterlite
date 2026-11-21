import subprocess
import sys
from typing import Final

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

import betterlite
from betterlite.llms.custom_llm import CustomLLM, CustomLLMError, EmbeddingResponse
from betterlite.types.utils import LiteLLMLoggingBaseClass

PACKAGES: Final = ("litellm", "betterlite")
NOT_IMPLEMENTED: Final = "Not implemented yet!"
SERVER_ERROR: Final = 500
UNAVAILABLE: Final = 503
CHUNK_COUNT: Final = 2
# LiteLLM names in these modules that betterlite has no counterpart for or that another task owns
SKIPPED_NAMES: Final[dict[str, frozenset[str]]] = {
    "custom_llm": frozenset({
        "Any", "AsyncHTTPHandler", "AsyncIterator", "Callable", "Coroutine", "HTTPHandler",
        "Iterator", "TYPE_CHECKING", "Union", "custom_chat_llm_router", "httpx",
    }),
    "base": frozenset({"Any", "TYPE_CHECKING", "Union", "httpx", "litellm"}),
    "types": frozenset({"TypedDict"}),
}
# strict mypy reads LiteLLM's bare list and dict annotations as Any; the probe runs in a subprocess
PROBE: Final = """
import asyncio
import importlib
import inspect
import json
import sys

package = sys.argv[1]
top = importlib.import_module(package)
custom = importlib.import_module(package + ".llms.custom_llm")
base = importlib.import_module(package + ".llms.base")
item = importlib.import_module(package + ".types.llms.custom_llm")
utils = importlib.import_module(package + ".types.utils")
METHODS = [
    "completion", "streaming", "acompletion", "astreaming", "image_generation",
    "aimage_generation", "embedding", "aembedding", "image_edit", "aimage_edit",
]
ERROR_CASES = [[500, "Not implemented yet!"], [404, ""], [0, "zero"], [429, "slow down"]]
USAGE = {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
CHUNKS = [
    {"text": "he", "tool_use": None, "is_finished": False, "finish_reason": "",
     "usage": None, "index": 0},
    {"text": "llo", "tool_use": None, "is_finished": True, "finish_reason": "stop",
     "usage": USAGE, "index": 0},
]


def error_view(error):
    return {
        "kind": type(error).__name__, "status_code": error.status_code,
        "message": error.message, "text": str(error), "shown": repr(error),
        "args": list(error.args),
    }


def finish(value):
    return asyncio.run(value) if inspect.iscoroutine(value) else value


def unimplemented(name):
    method = getattr(custom.CustomLLM(), name)
    parameters = inspect.signature(method).parameters.values()
    required = [p for p in parameters if p.default is inspect.Parameter.empty]
    try:
        finish(method(*[None] * len(required)))
    except custom.CustomLLMError as error:
        return error_view(error)
    return None


def signature(name):
    rows = []
    for p in inspect.signature(getattr(custom.CustomLLM, name)).parameters.values():
        has_default = p.default is not inspect.Parameter.empty
        rows.append([p.name, p.kind.name, has_default, p.default if has_default else None])
    return rows


class EchoLLM(custom.CustomLLM):
    def completion(self, *args, **kwargs):
        content = "echo " + kwargs["messages"][-1]["content"]
        message = {"role": "assistant", "content": content}
        choice = {"index": 0, "finish_reason": "stop", "message": message}
        return utils.ModelResponse(model=kwargs["model"], choices=[choice])

    async def acompletion(
        self, model, messages, api_base, custom_prompt_dict, model_response, print_verbose,
        encoding, api_key, logging_obj, optional_params, acompletion=None,
        litellm_params=None, logger_fn=None, headers={}, timeout=None, client=None,
    ):
        return self.completion(model=model, messages=messages)

    def streaming(self, *args, **kwargs):
        yield from CHUNKS

    async def astreaming(self, *args, **kwargs):
        for chunk in CHUNKS:
            yield chunk

    def embedding(
        self, model, input, model_response, print_verbose, logging_obj, optional_params,
        api_key=None, api_base=None, timeout=None, litellm_params=None,
    ):
        data = [
            {"object": "embedding", "index": index, "embedding": [float(len(text))]}
            for index, text in enumerate(input)
        ]
        return utils.EmbeddingResponse(model=model, data=data)

    async def aembedding(self, *args, **kwargs):
        return self.embedding(*args, **kwargs)


def chat_call(acompletion):
    return {
        "model": "echo-model", "messages": [{"role": "user", "content": "hello"}],
        "headers": {}, "model_response": utils.ModelResponse(), "print_verbose": print,
        "api_key": "sk-test", "api_base": "http://unused", "acompletion": acompletion,
        "logging_obj": None, "optional_params": {}, "litellm_params": {}, "logger_fn": None,
        "timeout": 600.0, "custom_prompt_dict": {}, "client": None, "encoding": None,
    }


def embedding_call():
    return {
        "model": "echo-embed", "input": ["a", "bcd"], "logging_obj": None,
        "api_base": "http://unused", "api_key": "sk-test", "timeout": 600.0,
        "optional_params": {}, "model_response": utils.EmbeddingResponse(),
        "print_verbose": print, "litellm_params": {},
    }


def chat_view(response):
    choice = response.choices[0]
    return {
        "model": response.model, "object_type": response.object,
        "role": choice.message.role, "content": choice.message.content,
        "finish_reason": choice.finish_reason,
    }


def embedding_view(response):
    return {"model": response.model, "object_type": response.object, "data": response.data}


async def collect(stream):
    return [chunk async for chunk in stream]


def handler_view():
    empty = top.custom_provider_map == []
    top.custom_provider_map = [item.CustomLLMItem(provider="echo", custom_handler=EchoLLM())]
    handler = None
    for entry in top.custom_provider_map:
        if entry["provider"] == "echo":
            handler = entry["custom_handler"]
    return {
        "provider_map_empty": empty,
        "completion": chat_view(handler.completion(**chat_call(False))),
        "acompletion": chat_view(asyncio.run(handler.acompletion(**chat_call(True)))),
        "streaming": list(handler.streaming(**chat_call(False))),
        "astreaming": asyncio.run(collect(handler.astreaming(**chat_call(True)))),
        "embedding": embedding_view(handler.embedding(**embedding_call())),
        "aembedding": embedding_view(asyncio.run(handler.aembedding(**embedding_call()))),
    }


class Session:
    def __init__(self, closed):
        self.closed = closed

    def close(self):
        self.closed.append("close")

    async def aclose(self):
        self.closed.append("aclose")


def base_view():
    llm = base.BaseLLM()
    marker = utils.ModelResponse()
    text_marker = utils.TextCompletionResponse()
    common = {
        "model": "m", "response": None, "stream": False, "logging_obj": None,
        "optional_params": {}, "api_key": "k", "data": {}, "messages": [],
        "print_verbose": print, "encoding": None,
    }
    positional = llm.process_response("m", None, marker, True, None, {}, "k", "", [], print, None)
    text = llm.process_text_completion_response(model_response=text_marker, **common)
    shared = {
        "process_response": llm.process_response(model_response=marker, **common) is marker,
        "positional": positional is marker,
        "text_completion": text is text_marker,
        "validate_empty": llm.validate_environment() is None,
        "validate_args": llm.validate_environment("a", 1, key="v") is None,
        "exit_without_session": llm.__exit__() is None,
        "aexit_without_session": asyncio.run(llm.__aexit__(None, None, None)) is None,
        "class_session": base.BaseLLM._client_session is None,
        "custom_is_base": issubclass(custom.CustomLLM, base.BaseLLM),
    }
    closed = []
    llm._client_session = Session(closed)
    shared["exit_result"] = llm.__exit__() is None
    llm._aclient_session = Session(closed)
    shared["aexit_result"] = asyncio.run(llm.__aexit__(None, None, None)) is None
    shared["closed"] = closed
    llm._aclient_session = None
    try:
        asyncio.run(llm.__aexit__(None, None, None))
        none_session = "returned"
    except AttributeError:
        none_session = "AttributeError"
    deviations = {
        "has_completion": hasattr(base.BaseLLM, "completion"),
        "has_embedding": hasattr(base.BaseLLM, "embedding"),
        "aexit_none_session": none_session,
    }
    return {"shared": shared, "deviations": deviations}


def public(module):
    return sorted(name for name in dir(module) if not name.startswith("_"))


def keys(typed_dict):
    return {
        "required": sorted(typed_dict.__required_keys__),
        "optional": sorted(typed_dict.__optional_keys__),
    }


print(json.dumps({
    "errors": [error_view(custom.CustomLLMError(*case)) for case in ERROR_CASES]
    + [error_view(custom.CustomLLMError(status_code=418, message="teapot"))],
    "signatures": {name: signature(name) for name in METHODS},
    "coroutines": {name: inspect.iscoroutinefunction(getattr(custom.CustomLLM, name))
                   for name in METHODS},
    "unimplemented": {name: unimplemented(name) for name in METHODS},
    "handler": handler_view(),
    "names": {"custom_llm": public(custom), "base": public(base), "types": public(item)},
    "item_keys": keys(item.CustomLLMItem),
    "chunk_keys": keys(custom.GenericStreamingChunk),
    "base": base_view(),
}))
"""


class ErrorView(TypedDict):
    kind: str
    status_code: int
    message: str
    text: str
    shown: str
    args: list[JsonValue]


class ChatView(TypedDict):
    model: str
    object_type: str
    role: str
    content: str
    finish_reason: str


class EmbeddingView(TypedDict):
    model: str
    object_type: str
    data: list[JsonValue]


class HandlerView(TypedDict):
    provider_map_empty: bool
    completion: ChatView
    acompletion: ChatView
    streaming: list[JsonValue]
    astreaming: list[JsonValue]
    embedding: EmbeddingView
    aembedding: EmbeddingView


class KeySets(TypedDict):
    required: list[str]
    optional: list[str]


class SharedBaseView(TypedDict):
    process_response: bool
    positional: bool
    text_completion: bool
    validate_empty: bool
    validate_args: bool
    exit_without_session: bool
    aexit_without_session: bool
    class_session: bool
    custom_is_base: bool
    exit_result: bool
    aexit_result: bool
    closed: list[str]


class BaseDeviations(TypedDict):
    has_completion: bool
    has_embedding: bool
    aexit_none_session: str


class BaseView(TypedDict):
    shared: SharedBaseView
    deviations: BaseDeviations


class Report(TypedDict):
    errors: list[ErrorView]
    signatures: dict[str, list[tuple[str, str, bool, JsonValue]]]
    coroutines: dict[str, bool]
    unimplemented: dict[str, ErrorView | None]
    handler: HandlerView
    names: dict[str, list[str]]
    item_keys: KeySets
    chunk_keys: KeySets
    base: BaseView


REPORT: Final = TypeAdapter(Report)
UNIMPLEMENTED_VIEW: Final = ErrorView(
    kind="CustomLLMError", status_code=SERVER_ERROR, message=NOT_IMPLEMENTED,
    text=NOT_IMPLEMENTED, shown=f"CustomLLMError('{NOT_IMPLEMENTED}')", args=[NOT_IMPLEMENTED],
)


@pytest.fixture(scope="module")
def reports() -> dict[str, Report]:
    found: dict[str, Report] = {}
    for package in PACKAGES:
        done = subprocess.run(
            [sys.executable, "-c", PROBE, package],
            capture_output=True,
            text=True,
            check=False,
        )
        assert done.returncode == 0, done.stderr
        found[package] = REPORT.validate_json(done.stdout)
    return found


def test_custom_llm_error_matches(reports: dict[str, Report]) -> None:
    assert reports["betterlite"]["errors"] == reports["litellm"]["errors"]
    assert reports["betterlite"]["errors"][0] == UNIMPLEMENTED_VIEW


def test_custom_llm_error_typed() -> None:
    error = CustomLLMError(status_code=UNAVAILABLE, message="down")
    assert error.status_code == UNAVAILABLE
    assert error.message == "down"
    assert str(error) == "down"


def test_custom_llm_surface_params(reports: dict[str, Report]) -> None:
    assert reports["betterlite"]["signatures"] == reports["litellm"]["signatures"]
    assert reports["betterlite"]["coroutines"] == reports["litellm"]["coroutines"]
    assert len(reports["betterlite"]["signatures"]) == len(reports["betterlite"]["coroutines"])


def test_custom_llm_surface_raises(reports: dict[str, Report]) -> None:
    ours = reports["betterlite"]["unimplemented"]
    assert ours == reports["litellm"]["unimplemented"]
    assert sorted(ours) == sorted(reports["betterlite"]["signatures"])
    assert all(view == UNIMPLEMENTED_VIEW for view in ours.values())


def test_custom_llm_surface_typed() -> None:
    with pytest.raises(CustomLLMError) as caught:
        CustomLLM().embedding(
            model="m", input=["hi"], model_response=EmbeddingResponse(),
            print_verbose=print, logging_obj=LiteLLMLoggingBaseClass(), optional_params={},
        )
    assert caught.value.status_code == SERVER_ERROR
    assert caught.value.message == NOT_IMPLEMENTED


def test_custom_llm_surface_runs(reports: dict[str, Report]) -> None:
    ours = reports["betterlite"]["handler"]
    assert ours == reports["litellm"]["handler"]
    assert ours["completion"]["content"] == "echo hello"
    assert ours["acompletion"] == ours["completion"]
    assert len(ours["streaming"]) == len(ours["astreaming"]) == CHUNK_COUNT
    assert ours["embedding"]["model"] == "echo-embed"
    assert ours["aembedding"] == ours["embedding"]


def test_custom_llm_surface_map(reports: dict[str, Report]) -> None:
    assert len(betterlite.custom_provider_map) == 0
    assert reports["betterlite"]["handler"]["provider_map_empty"]
    assert reports["litellm"]["handler"]["provider_map_empty"]


def test_custom_llm_surface_names(reports: dict[str, Report]) -> None:
    theirs = reports["litellm"]["names"]
    ours = reports["betterlite"]["names"]
    assert sorted(ours) == sorted(SKIPPED_NAMES)
    for module, skipped in SKIPPED_NAMES.items():
        assert ours[module] == sorted(set(theirs[module]) - skipped)


def test_custom_llm_surface_keys(reports: dict[str, Report]) -> None:
    assert reports["betterlite"]["item_keys"] == reports["litellm"]["item_keys"]
    assert reports["betterlite"]["item_keys"] == KeySets(
        required=["custom_handler", "provider"], optional=[],
    )
    assert reports["betterlite"]["chunk_keys"] == reports["litellm"]["chunk_keys"]


def test_base_llm_matches(reports: dict[str, Report]) -> None:
    ours = reports["betterlite"]["base"]["shared"]
    assert ours == reports["litellm"]["base"]["shared"]
    assert ours == SharedBaseView(
        process_response=True, positional=True, text_completion=True, validate_empty=True,
        validate_args=True, exit_without_session=True, aexit_without_session=True,
        class_session=True, custom_is_base=True, exit_result=True, aexit_result=True,
        closed=["close", "aclose"],
    )


def test_base_llm_deviations(reports: dict[str, Report]) -> None:
    assert reports["litellm"]["base"]["deviations"] == BaseDeviations(
        has_completion=True, has_embedding=True, aexit_none_session="AttributeError",
    )
    assert reports["betterlite"]["base"]["deviations"] == BaseDeviations(
        has_completion=False, has_embedding=False, aexit_none_session="returned",
    )
