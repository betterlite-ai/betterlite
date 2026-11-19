import json
import logging
import os
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from typing import Final, NotRequired, Protocol, TypedDict, Unpack, cast

import litellm
from litellm.exceptions import AuthenticationError
from litellm.llms.anthropic.chat.handler import ModelResponseIterator
from litellm.llms.bedrock.claude_platform.messages_transformation import (
    BedrockClaudePlatformMessagesConfig,
)
from litellm.llms.bedrock.claude_platform.transformation import BedrockClaudePlatformConfig
from pydantic import BaseModel, ConfigDict, JsonValue as Json, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

BETA_HEADER: Final = "anthropic-beta"
WORKSPACE_HEADER: Final = "anthropic-workspace-id"
DROP_MARKER: Final = "dropping unsupported Messages API param(s) "
IGNORE_MARKER: Final = "ignoring claude_platform_unsupported_params of type "
BILLING_MARKER: Final = "x-anthropic-billing-header"
AUTHENTICATION_PREFIX: Final = "litellm.AuthenticationError: "
EMPTY_PARAMS: Final[dict[str, Json]] = {}
EMPTY_STRINGS: Final[dict[str, str]] = {}
EMPTY_MESSAGES: Final[list[Json]] = []
MANAGED_ENV: Final = (
    "ANTHROPIC_AWS_WORKSPACE_ID",
    "ANTHROPIC_WORKSPACE_ID",
    "ANTHROPIC_AWS_BASE_URL",
    "ANTHROPIC_AWS_API_BASE",
    "ANTHROPIC_AWS_API_KEY",
)

PAIR_ADAPTER: Final[TypeAdapter[list[Json]]] = TypeAdapter(list[Json])


class WorkspaceFields(TypedDict):
    optional_params: NotRequired[dict[str, Json]]
    deployment: NotRequired[dict[str, str]]
    litellm_settings: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]


class WorkspaceInput(TypedDict):
    model: str
    cases: list[WorkspaceFields]


class HeaderInput(WorkspaceFields):
    model: str
    messages: NotRequired[list[Json]]
    optional_params: NotRequired[dict[str, Json]]
    litellm_params: NotRequired[dict[str, Json]]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class BodyInput(TypedDict):
    model: str
    messages: list[Json]
    optional_params: NotRequired[dict[str, Json]]
    litellm_params: NotRequired[dict[str, Json]]
    headers: NotRequired[dict[str, str]]


class StreamInput(TypedDict):
    id: str
    json_mode: NotRequired[bool | None]
    lines: list[str]


class Input(TypedDict):
    workspace_id_required: NotRequired[WorkspaceInput]
    chat_platform_headers: NotRequired[HeaderInput]
    native_platform_headers: NotRequired[HeaderInput]
    chat_platform_body: NotRequired[BodyInput]
    native_platform_body: NotRequired[BodyInput]
    native_anthropic_stream: NotRequired[StreamInput]


class ValidateArguments(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class ChatValidator(Protocol):
    def __call__(self, **kwargs: Unpack[ValidateArguments]) -> dict[str, str]: ...


class NativeValidator(Protocol):
    def __call__(
        self, **kwargs: Unpack[ValidateArguments]
    ) -> tuple[dict[str, str], str | None]: ...


class ChatTransformArguments(TypedDict):
    model: str
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    headers: dict[str, str]


class NativeTransformArguments(TypedDict):
    model: str
    messages: list[Json]
    anthropic_messages_optional_request_params: dict[str, Json]
    litellm_params: dict[str, Json]
    headers: dict[str, str]


class ChatTransformer(Protocol):
    def __call__(self, **kwargs: Unpack[ChatTransformArguments]) -> dict[str, Json]: ...


class NativeTransformer(Protocol):
    def __call__(self, **kwargs: Unpack[NativeTransformArguments]) -> dict[str, Json]: ...


class LineSource(Protocol):
    def __iter__(self) -> "LineSource": ...

    def __next__(self) -> str: ...


class IteratorArguments(TypedDict):
    streaming_response: LineSource
    sync_stream: bool
    json_mode: bool | None


class IteratorFactory(Protocol):
    def __call__(self, **kwargs: Unpack[IteratorArguments]) -> "SyncSource": ...


class HasDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncSource(Protocol):
    response_id: str

    def __iter__(self) -> "SyncSource": ...

    def __next__(self) -> HasDump: ...


class SystemBlock(TypedDict):
    type: str
    text: NotRequired[Json]


class ChunkDelta(TypedDict):
    content: Json


class ChunkChoice(TypedDict):
    delta: ChunkDelta
    finish_reason: Json


class ChunkView(TypedDict):
    id: str
    model: Json
    choices: list[ChunkChoice]


SYSTEM_BLOCKS: Final[TypeAdapter[list[SystemBlock]]] = TypeAdapter(list[SystemBlock])
CHUNK_VIEW: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)

CHAT_CONFIG: Final = BedrockClaudePlatformConfig()
NATIVE_CONFIG: Final = BedrockClaudePlatformMessagesConfig()
CHAT_VALIDATE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/transformation.py#L27
    ChatValidator, CHAT_CONFIG.validate_environment
)
NATIVE_VALIDATE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/messages_transformation.py#L23
    NativeValidator, NATIVE_CONFIG.validate_anthropic_messages_environment
)
CHAT_TRANSFORM: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/transformation.py#L73
    ChatTransformer, CHAT_CONFIG.transform_request
)
NATIVE_TRANSFORM: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/messages_transformation.py#L69
    NativeTransformer, NATIVE_CONFIG.transform_anthropic_messages_request
)
STREAM_ITERATOR: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/transformation.py#L92
    IteratorFactory, CHAT_CONFIG.get_model_response_iterator
)


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int
    message: str
    llm_provider: str
    model: str | None


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.entries: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.entries.append(warning_entry(record.getMessage()))


@contextmanager
def captured_warnings() -> Generator[list[Json], None, None]:
    handler = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(handler)
    try:
        yield handler.entries
    finally:
        logger.removeHandler(handler)


def warning_entry(message: str) -> Json:
    if DROP_MARKER in message:
        _, _, listed = message.partition(DROP_MARKER)
        tuple_text, _, _ = listed.partition(" from the request body")
        inner = tuple_text.strip()[1:-1].rstrip(",")
        names: list[Json] = (
            [] if not inner else [name.strip("'\"") for name in inner.split(", ")]
        )
        return {"action": "drop", "params": names}
    _, _, type_text = message.partition(IGNORE_MARKER)
    return {"action": "ignore", "type_name": type_text.split(";", 1)[0]}


def failure_view(error: AuthenticationError) -> Json:
    failure = Failure.model_validate(error, from_attributes=True)
    model = failure.model if failure.model is not None else ""
    return {
        "status": failure.status_code,
        "provider": failure.llm_provider,
        "model": model,
        "message": failure.message.removeprefix(AUTHENTICATION_PREFIX),
    }


def apply_environment(environment: dict[str, str] | None) -> None:
    # Each case's workspace values replace the previous case's, so no value leaks between them.
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    if environment is not None:
        os.environ.update(environment)


def merged_params(case: WorkspaceFields) -> dict[str, Json]:
    merged: dict[str, Json] = {}
    settings = case.get("litellm_settings")
    if settings is not None:
        for key, value in settings.items():
            merged[key] = value
    deployment = case.get("deployment")
    if deployment is not None:
        for key, value in deployment.items():
            merged[key] = value
    return merged


def params_of(optional: dict[str, Json] | None) -> dict[str, Json]:
    if optional is None:
        return EMPTY_PARAMS
    return optional


def beta_tokens(value: str) -> list[Json]:
    tokens: set[str] = {piece.strip() for piece in value.split(",") if piece.strip()}
    ordered: list[Json] = []
    for token in sorted(tokens):
        ordered.append(token)
    return ordered


def beta_view(headers: Mapping[str, str]) -> list[Json]:
    value = headers.get(BETA_HEADER)
    if value is None:
        return []
    return beta_tokens(value)


def project_headers(headers: Mapping[str, str]) -> list[Json]:
    projected: list[Json] = []
    for name in sorted(headers):
        value: Json = headers[name]
        if name == BETA_HEADER:
            value = beta_tokens(headers[name])
        projected.append(PAIR_ADAPTER.validate_python([name, value]))
    return projected


def system_texts(system: Json) -> list[Json]:
    if system is None:
        return []
    texts: list[Json] = []
    for block in SYSTEM_BLOCKS.validate_python(system):
        if block["type"] != "text":
            continue
        text = block.get("text")
        if text is not None:
            texts.append(text)
    return texts


def body_view(
    body: dict[str, Json], warnings: list[Json], headers: Mapping[str, str]
) -> Json:
    return {
        "model": body.get("model"),
        "max_tokens": body.get("max_tokens"),
        "workspace_removed": "workspace_id" not in body,
        "context_removed": "context_management" not in body,
        "override_removed": "claude_platform_unsupported_params" not in body,
        "system_texts": system_texts(body.get("system")),
        "billing": BILLING_MARKER in json.dumps(body),
        "beta": beta_view(headers),
        "warnings": warnings,
    }


def workspace_view(headers: Mapping[str, str]) -> Json:
    workspace: Json = headers[WORKSPACE_HEADER]
    return {"workspace": workspace}


def chat_workspace(case: WorkspaceFields, model: str) -> Json:
    apply_environment(case.get("environment"))
    try:
        headers = CHAT_VALIDATE(
            headers=EMPTY_STRINGS,
            model=model,
            messages=EMPTY_MESSAGES,
            optional_params=params_of(case.get("optional_params")),
            litellm_params=merged_params(case),
        )
    except AuthenticationError as error:
        return failure_view(error)
    return workspace_view(headers)


def native_workspace(case: WorkspaceFields, model: str) -> Json:
    apply_environment(case.get("environment"))
    try:
        headers, _api_base = NATIVE_VALIDATE(
            headers=EMPTY_STRINGS,
            model=model,
            messages=EMPTY_MESSAGES,
            optional_params=params_of(case.get("optional_params")),
            litellm_params=merged_params(case),
        )
    except AuthenticationError as error:
        return failure_view(error)
    return workspace_view(headers)


def workspace_answers(input_value: WorkspaceInput) -> Json:
    model = input_value["model"]
    outcomes: list[Json] = []
    for case in input_value["cases"]:
        outcomes.append(
            {
                "chat": chat_workspace(case, model),
                "native": native_workspace(case, model),
            }
        )
    return {"outcomes": outcomes}


def chat_headers(input_value: HeaderInput) -> Json:
    params = input_value.get("optional_params")
    params = EMPTY_PARAMS if params is None else params
    messages = input_value.get("messages")
    messages = EMPTY_MESSAGES if messages is None else messages
    headers = input_value.get("headers")
    headers = EMPTY_STRINGS if headers is None else headers
    try:
        validated = CHAT_VALIDATE(
            headers=headers,
            model=input_value["model"],
            messages=messages,
            optional_params=params,
            litellm_params=merged_params(input_value),
            api_key=input_value.get("api_key"),
            api_base=input_value.get("api_base"),
        )
    except AuthenticationError as error:
        return failure_view(error)
    return {"headers": project_headers(validated)}


def native_headers(input_value: HeaderInput) -> Json:
    params = input_value.get("optional_params")
    params = EMPTY_PARAMS if params is None else params
    messages = input_value.get("messages")
    messages = EMPTY_MESSAGES if messages is None else messages
    headers = input_value.get("headers")
    headers = EMPTY_STRINGS if headers is None else headers
    # A deployment's own parameters reach the provider as litellm_params, after the global settings.
    litellm_params = merged_params(input_value)
    explicit = input_value.get("litellm_params")
    if explicit is not None:
        for key, value in explicit.items():
            litellm_params[key] = value
    try:
        validated, api_base = NATIVE_VALIDATE(
            headers=headers,
            model=input_value["model"],
            messages=messages,
            optional_params=params,
            litellm_params=litellm_params,
            api_key=input_value.get("api_key"),
            api_base=input_value.get("api_base"),
        )
    except AuthenticationError as error:
        return failure_view(error)
    return {"headers": project_headers(validated), "api_base": api_base}


def chat_body(input_value: BodyInput) -> Json:
    params = params_of(input_value.get("optional_params"))
    litellm_params = input_value.get("litellm_params")
    litellm_params = EMPTY_PARAMS if litellm_params is None else litellm_params
    headers = input_value.get("headers")
    headers = EMPTY_STRINGS if headers is None else headers
    working_headers = dict(headers)
    with captured_warnings() as entries:
        body = CHAT_TRANSFORM(
            model=input_value["model"],
            messages=list(input_value["messages"]),
            optional_params=dict(params),
            litellm_params=dict(litellm_params),
            headers=working_headers,
        )
    return body_view(body, entries, working_headers)


def native_body(input_value: BodyInput) -> Json:
    params = params_of(input_value.get("optional_params"))
    litellm_params = input_value.get("litellm_params")
    litellm_params = EMPTY_PARAMS if litellm_params is None else litellm_params
    working_headers: dict[str, str] = {}
    with captured_warnings() as entries:
        body = NATIVE_TRANSFORM(
            model=input_value["model"],
            messages=list(input_value["messages"]),
            anthropic_messages_optional_request_params=dict(params),
            litellm_params=dict(litellm_params),
            headers=working_headers,
        )
    return body_view(body, entries, working_headers)


def chunk_view(item: HasDump) -> Json:
    view = CHUNK_VIEW.validate_json(item.model_dump_json())
    model = view["model"] if view["model"] is not None else ""
    choices = view["choices"]
    if not choices:
        return {"id": view["id"], "model": model, "content": "", "finish_reason": None}
    choice = choices[0]
    content = choice["delta"]["content"] if choice["delta"]["content"] is not None else ""
    return {
        "id": view["id"],
        "model": model,
        "content": content,
        "finish_reason": choice["finish_reason"],
    }


class LineChunks:
    def __init__(self, lines: list[str]) -> None:
        self.lines = iter(lines)

    def __iter__(self) -> "LineChunks":
        return self

    def __next__(self) -> str:
        return next(self.lines)


def stream_answer(input_value: StreamInput) -> Json:
    json_mode = input_value.get("json_mode")
    source = STREAM_ITERATOR(
        streaming_response=LineChunks(input_value["lines"]),
        sync_stream=False,
        json_mode=json_mode,
    )
    source.response_id = "chatcmpl-" + input_value["id"]
    chunks: list[Json] = []
    for item in source:
        chunks.append(chunk_view(item))
    return {"chunks": chunks}


def run(input_value: Input) -> Json:
    if "workspace_id_required" in input_value:
        return workspace_answers(input_value["workspace_id_required"])
    if "chat_platform_headers" in input_value:
        return chat_headers(input_value["chat_platform_headers"])
    if "native_platform_headers" in input_value:
        return native_headers(input_value["native_platform_headers"])
    if "chat_platform_body" in input_value:
        return chat_body(input_value["chat_platform_body"])
    if "native_platform_body" in input_value:
        return native_body(input_value["native_platform_body"])
    if "native_anthropic_stream" in input_value:
        return stream_answer(input_value["native_anthropic_stream"])
    raise ValueError("the input names exactly one claude platform variant")
