import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack, runtime_checkable

import litellm
from litellm import main
from litellm.exceptions import APIError
from litellm.llms.azure_ai import agents as agents_module
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter, ValidationError


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None


class Configured(TypedDict):
    api_base: str | None
    api_key: str | None
    globals: Globals
    environment: dict[str, str]


class RewriteInput(TypedDict):
    op: Literal["rewrite"]
    base: str


class EmbeddingInput(Configured):
    op: Literal["embedding"]


class DispatchInput(Configured):
    op: Literal["dispatch"]
    model: str
    messages: list[dict[str, JsonValue]]
    headers: dict[str, str] | None
    stream: bool
    status: int | None


Input = RewriteInput | EmbeddingInput | DispatchInput


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
    input: NotRequired[list[str]]
    messages: NotRequired[list[dict[str, JsonValue]]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    headers: NotRequired[dict[str, str] | None]
    custom_llm_provider: NotRequired[str]


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
    client: None = None
    custom_llm_provider: str = "azure_ai"
    extra_headers: dict[str, str] | None = None
    shared_session: str | None = None
    timeout: int = 5
    logger_fn: None = None


class ContextFactory(Protocol):
    def __call__(self, **_kwargs: JsonValue) -> Context: ...


@runtime_checkable
class Handler(Protocol):
    def completion(self, **kwargs: Unpack[Arguments]) -> ModelResponse: ...
    def embedding(self, **kwargs: Unpack[Arguments]) -> EmbeddingResponse: ...


class ChatCall(Protocol):
    def __call__(self, **kwargs: Unpack[Arguments]) -> ModelResponse: ...


class AgentsConfig(Protocol):
    completion: ChatCall


@runtime_checkable
class AgentsModule(Protocol):
    AzureAIAgentsConfig: AgentsConfig


@runtime_checkable
class MainFields(Protocol):
    _CompletionDispatchContext: ContextFactory
    _get_encoding: Callable[[], str]
    base_llm_http_handler: Handler
    azure_anthropic_chat_completions: Handler  # ast-grep-ignore: no-long-names
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
        self.failed = False
        self.status: int | None = None

    def post_call(self, **kwargs: Unpack[PostArguments]) -> None:
        logged_headers = kwargs.get("additional_args") is not None
        match kwargs["original_response"]:
            case str():
                self.post_calls.append(PostCall(error=True, headers=logged_headers))
            case ModelResponse():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181
                self.post_calls.append(PostCall(error=False, headers=logged_headers))

    def record(self, branch: str, kwargs: Arguments) -> None:
        self.branch = branch
        self.base = kwargs.get("api_base")
        self.key = kwargs.get("api_key")
        self.headers = kwargs.get("headers")
        self.messages = normalize(WIRE.validate_python(kwargs.get("messages")))


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

    def embedding(self, **kwargs: Unpack[Arguments]) -> EmbeddingResponse:
        recorder = self.recorder
        recorder.branch = "embedding"
        recorder.base = kwargs.get("api_base")
        recorder.key = kwargs.get("api_key")
        recorder.headers = kwargs.get("headers")
        return EmbeddingResponse()


class FixedContext:
    def __init__(self, context: Context) -> None:
        self.context = context

    def __call__(self, **_kwargs: JsonValue) -> Context:
        return self.context


class NativeBindings:
    def __init__(self, fields: MainFields) -> None:
        self.fields = fields

    @contextmanager
    def install(self, context: Context, recorder: Recorder) -> Generator[None]:
        fields = self.fields
        factory = fields._CompletionDispatchContext  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1617
        encoding = fields._get_encoding  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1617
        chat = fields.base_llm_http_handler
        claude = fields.azure_anthropic_chat_completions
        embedding = fields.azure_ai_embedding
        agents = AGENTS.AzureAIAgentsConfig.completion
        leaf = RecorderLeaf(recorder, "chat")
        fields._CompletionDispatchContext = FixedContext(context)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1617
        fields._get_encoding = str  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1617
        fields.base_llm_http_handler = leaf
        fields.azure_anthropic_chat_completions = RecorderLeaf(recorder, "claude")
        fields.azure_ai_embedding = leaf
        AGENTS.AzureAIAgentsConfig.completion = RecorderLeaf(recorder, "agents").completion
        try:
            yield
        finally:
            fields._CompletionDispatchContext = factory  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1617
            fields._get_encoding = encoding  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1617
            fields.base_llm_http_handler = chat
            fields.azure_anthropic_chat_completions = claude
            fields.azure_ai_embedding = embedding
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


def request_context(supplied: DispatchInput, recorder: Recorder) -> Context:
    return Context(
        supplied["api_base"], supplied["api_key"], supplied["headers"],
        {}, recorder, supplied["messages"], supplied["model"],
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
                return WIRE.validate_json(value)
            except ValidationError:
                return value
        case list():
            return [normalize(item) for item in value]
        case dict():
            return {key: normalize(item) for key, item in value.items()}
        case None | bool() | int() | float():
            return value
