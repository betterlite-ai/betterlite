import json
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack, runtime_checkable

import litellm
from litellm import main
from litellm.exceptions import APIError
from litellm.llms.azure_ai import agents as agents_module
from litellm.types.utils import EmbeddingResponse, ModelResponse
from openai import OpenAIError
from pydantic import ConfigDict, JsonValue, TypeAdapter


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None


class Input(TypedDict):
    op: Literal["dispatch", "embedding", "rewrite"]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    base: NotRequired[str]
    environment: NotRequired[dict[str, str]]
    globals: NotRequired[Globals]
    headers: NotRequired[dict[str, str] | None]
    messages: NotRequired[list[dict[str, JsonValue]]]
    model: NotRequired[str]
    status: NotRequired[int | None]
    stream: NotRequired[bool]


class Case(TypedDict):
    inputs: list[Input]


class PostCall(TypedDict):
    error: bool
    headers: bool


class Observation(TypedDict):
    branch: str | None
    base: str | None
    key: str | None
    headers: dict[str, str] | None
    messages: JsonValue
    post_calls: list[PostCall]
    errors: bool


class Arguments(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None


class EmbeddingArguments(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: NotRequired[dict[str, str] | None]


class PostArguments(TypedDict):
    original_response: ModelResponse | str
    additional_args: NotRequired[dict[str, JsonValue]]


class PostRecorder(Protocol):
    def post_call(self, **kwargs: Unpack[PostArguments]) -> None: ...


@dataclass
class Context:
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None
    litellm_params: dict[str, JsonValue]
    logging: PostRecorder
    messages: list[dict[str, JsonValue]]
    model: str
    model_response: ModelResponse
    optional_params: dict[str, JsonValue]
    stream: bool = False
    acompletion: bool = False
    client: object | None = None
    custom_llm_provider: str = "azure_ai"
    extra_headers: dict[str, str] | None = None
    shared_session: str | None = None
    timeout: int = 5
    logger_fn: None = None


class ContextFactory(Protocol):
    def __call__(self, **_kwargs: object) -> Context: ...


class FixedContext:
    def __init__(self, context: Context) -> None:
        self.context = context

    def __call__(self, **_kwargs: object) -> Context:
        return self.context


@runtime_checkable
class Handler(Protocol):
    completion: Callable[..., ModelResponse]
    embedding: Callable[..., EmbeddingResponse]


@runtime_checkable
class AgentsConfig(Protocol):
    completion: Callable[..., ModelResponse]


@runtime_checkable
class AgentsModule(Protocol):
    AzureAIAgentsConfig: AgentsConfig


@runtime_checkable
class MainFields(Protocol):
    _CompletionDispatchContext: ContextFactory
    _get_encoding: Callable[[], str]
    base_llm_http_handler: Handler
    azure_anthropic_chat_completions: Handler
    azure_ai_embedding: Handler


@runtime_checkable
class GlobalFields(Protocol):
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None


CONFIG: Final = ConfigDict(arbitrary_types_allowed=True)
MAIN: Final = TypeAdapter(MainFields, config=CONFIG).validate_python(main)
AGENTS: Final = TypeAdapter(AgentsModule, config=CONFIG).validate_python(agents_module)
GLOBALS: Final = TypeAdapter(GlobalFields, config=CONFIG).validate_python(litellm)
PUBLIC: Final = TypeAdapter(Handler, config=CONFIG).validate_python(litellm)
WIRE: Final = TypeAdapter(JsonValue)
EMPTY: Final = Globals(api_base=None, api_key=None, headers=None)
CHAT_MODEL: Final = "gpt-4o"
CLAUDE_MODEL: Final = "claude-3-haiku-20240307"
EMBED_MODEL: Final = "text-embedding-3-small"
FAILURE_STATUS: Final = 400
MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": "hi"}]


class Recorder:
    def __init__(self) -> None:
        self.branch: str | None = None
        self.base: str | None = None
        self.key: str | None = None
        self.headers: dict[str, str] | None = None
        self.messages: JsonValue = []
        self.post_calls: list[PostCall] = []
        self.status: int | None = None

    def post_call(self, **kwargs: Unpack[PostArguments]) -> None:
        self.post_calls.append(
            PostCall(
                error=isinstance(kwargs["original_response"], str),
                headers=kwargs.get("additional_args") is not None,
            )
        )

    def record(self, branch: str, kwargs: Arguments) -> None:
        self.branch, self.base, self.key, self.headers = (
            branch, kwargs["api_base"], kwargs["api_key"], kwargs["headers"],
        )
        self.messages = normalize(WIRE.validate_python(kwargs["messages"]))


class RecorderLeaf:
    def __init__(self, recorder: Recorder, branch: str) -> None:
        self.recorder = recorder
        self.branch = branch

    def completion(self, **kwargs: Unpack[Arguments]) -> ModelResponse:
        self.recorder.record(self.branch, kwargs)
        failure = self.recorder.status
        if self.branch == "chat" and failure is not None and failure >= FAILURE_STATUS:
            raise APIError(
                status_code=failure, message="fixture failure", model=kwargs["model"],
                llm_provider="azure_ai",
            )
        return ModelResponse(id="chatcmpl-oracle", created=1_700_000_000, model=kwargs["model"])

    def embedding(self, **kwargs: Unpack[EmbeddingArguments]) -> EmbeddingResponse:
        recorder = self.recorder
        recorder.branch, recorder.base, recorder.key = (
            "embedding", kwargs["api_base"], kwargs["api_key"],
        )
        recorder.headers = kwargs.get("headers")
        return EmbeddingResponse()


class NativeBindings(MainFields):
    @contextmanager
    def install(self: MainFields, context: Context, recorder: Recorder) -> Generator[None]:
        context_factory = self._CompletionDispatchContext
        encoding = self._get_encoding
        chat = self.base_llm_http_handler
        claude = self.azure_anthropic_chat_completions
        embedding = self.azure_ai_embedding
        agents = AGENTS.AzureAIAgentsConfig.completion
        leaf = RecorderLeaf(recorder, "chat")
        self._CompletionDispatchContext = FixedContext(context)
        self._get_encoding = partial(str, "cl100k_base")
        self.base_llm_http_handler = leaf
        self.azure_anthropic_chat_completions = RecorderLeaf(recorder, "claude")
        self.azure_ai_embedding = leaf
        AGENTS.AzureAIAgentsConfig.completion = RecorderLeaf(recorder, "agents").completion
        try:
            yield
        finally:
            self._CompletionDispatchContext = context_factory
            self._get_encoding = encoding
            self.base_llm_http_handler = chat
            self.azure_anthropic_chat_completions = claude
            self.azure_ai_embedding = embedding
            AGENTS.AzureAIAgentsConfig.completion = agents


@contextmanager
def environment(globals_case: Globals, values: dict[str, str]) -> Generator[None]:
    original_env = dict(os.environ)
    original = (GLOBALS.api_base, GLOBALS.api_key, GLOBALS.headers)
    os.environ.clear()
    os.environ.update(values | {"LITELLM_RUST": "false", "LITELLM_LOCAL_MODEL_COST_MAP": "true"})
    GLOBALS.api_base = globals_case["api_base"]
    GLOBALS.api_key = globals_case["api_key"]
    GLOBALS.headers = globals_case["headers"]
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(original_env)
        GLOBALS.api_base, GLOBALS.api_key, GLOBALS.headers = original


def context(supplied: Input, recorder: Recorder) -> Context:
    return Context(
        supplied["api_base"], supplied["api_key"], supplied["headers"], {},
        recorder, supplied["messages"], supplied["model"],
        ModelResponse(id="chatcmpl-input"),
        {"stream": True} if supplied["stream"] else {}, supplied["stream"],
    )


def frozen_context(model: str, api_base: str | None, recorder: Recorder) -> Context:
    return Context(
        api_base, None, None, {}, recorder, MESSAGES, model,
        ModelResponse(id="chatcmpl-input"), {},
    )


def normalize(value: JsonValue) -> JsonValue:
    match value:
        case str():
            try:
                return WIRE.validate_python(json.loads(value))
            except json.JSONDecodeError:
                return value
        case list():
            return [normalize(item) for item in value]
        case dict():
            return {key: normalize(item) for key, item in value.items()}
        case _:
            return value


def observation(recorder: Recorder) -> Observation:
    return Observation(
        branch=recorder.branch, base=recorder.base, key=recorder.key,
        headers=recorder.headers, messages=recorder.messages,
        post_calls=recorder.post_calls, errors=False,
    )


def failed() -> Observation:
    return Observation(
        branch=None, base=None, key=None, headers=None, messages=[], post_calls=[], errors=True,
    )


def observe(supplied: Input) -> Observation:
    recorder = Recorder()
    if supplied["op"] == "rewrite":
        with environment(EMPTY, {}), NativeBindings.install(
            MAIN, frozen_context(CLAUDE_MODEL, supplied["base"], recorder), recorder,
        ):
            try:
                PUBLIC.completion(
                    model=CHAT_MODEL, messages=MESSAGES, custom_llm_provider="azure_ai",
                )
            except (ValueError, OpenAIError):
                return failed()
        return observation(recorder)
    if supplied["op"] == "embedding":
        with environment(supplied["globals"], supplied["environment"]), NativeBindings.install(
            MAIN, frozen_context(EMBED_MODEL, None, recorder), recorder,
        ):
            try:
                PUBLIC.embedding(
                    model=EMBED_MODEL, input=["hello"], custom_llm_provider="azure_ai",
                    api_base=supplied["api_base"], api_key=supplied["api_key"],
                )
            except ValueError:
                return failed()
        return observation(recorder)
    recorder.status = supplied["status"]
    with environment(supplied["globals"], supplied["environment"]), NativeBindings.install(
        MAIN, context(supplied, recorder), recorder,
    ):
        try:
            PUBLIC.completion(model=CHAT_MODEL, messages=MESSAGES, custom_llm_provider="azure_ai")
        except ValueError:
            return failed()
        except OpenAIError:
            if recorder.branch is None:
                return failed()
    return observation(recorder)


def run(input_value: Case) -> JsonValue:
    litellm.suppress_debug_info = True
    results: list[Observation] = []
    for supplied in input_value["inputs"]:
        results.append(observe(supplied))
    return WIRE.validate_python(results)
