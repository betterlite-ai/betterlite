from typing import Final, Literal, cast

from litellm.llms.bedrock.claude_platform.messages_transformation import (
    BedrockClaudePlatformMessagesConfig,
)
from litellm.llms.bedrock.claude_platform.transformation import BedrockClaudePlatformConfig
from pydantic import JsonValue as Json

from .types import (
    AUTHENTICATION_ERROR,
    EMPTY_MESSAGES,
    EMPTY_PARAMS,
    EMPTY_STRINGS,
    BodyInput,
    ChatTransformer,
    ChatValidator,
    HeaderInput,
    Input,
    IteratorFactory,
    NativeTransformer,
    NativeValidator,
    StreamInput,
    WorkspaceFields,
    WorkspaceInput,
)
from .views import (
    apply_environment,
    body_view,
    captured_warnings,
    chunk_view,
    failure_view,
    merged_params,
    params_of,
    project_headers,
    workspace_view,
)

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


def workspace_answer(
    case: WorkspaceFields, model: str, kind: Literal["chat", "native"]
) -> Json:
    apply_environment(case.get("environment"))
    params = params_of(case.get("optional_params"))
    litellm_params = merged_params(case)
    try:
        if kind == "native":
            headers, _api_base = NATIVE_VALIDATE(
                headers=EMPTY_STRINGS,
                model=model,
                messages=EMPTY_MESSAGES,
                optional_params=params,
                litellm_params=litellm_params,
            )
        else:
            headers = CHAT_VALIDATE(
                headers=EMPTY_STRINGS,
                model=model,
                messages=EMPTY_MESSAGES,
                optional_params=params,
                litellm_params=litellm_params,
            )
    except AUTHENTICATION_ERROR as error:
        return failure_view(error)
    return workspace_view(headers)


def workspace_answers(input_value: WorkspaceInput) -> Json:
    model = input_value["model"]
    outcomes: list[Json] = [
        {
            "chat": workspace_answer(case, model, "chat"),
            "native": workspace_answer(case, model, "native"),
        }
        for case in input_value["cases"]
    ]
    return {"outcomes": outcomes}


def header_inputs(
    input_value: HeaderInput,
) -> tuple[dict[str, Json], list[Json], dict[str, str]]:
    params = params_of(input_value.get("optional_params"))
    messages = input_value.get("messages")
    messages = EMPTY_MESSAGES if messages is None else messages
    headers = input_value.get("headers")
    headers = EMPTY_STRINGS if headers is None else headers
    return params, messages, headers


def chat_headers(input_value: HeaderInput) -> Json:
    params, messages, headers = header_inputs(input_value)
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
    except AUTHENTICATION_ERROR as error:
        return failure_view(error)
    return {"headers": project_headers(validated)}


def native_headers(input_value: HeaderInput) -> Json:
    params, messages, headers = header_inputs(input_value)
    # A deployment's own parameters reach the provider as litellm_params, after the global settings.
    litellm_params = merged_params(input_value)
    explicit = input_value.get("litellm_params")
    if explicit is not None:
        litellm_params.update(explicit)
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
    except AUTHENTICATION_ERROR as error:
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
            input_value["model"],
            list(input_value["messages"]),
            dict(params),
            dict(litellm_params),
            working_headers,
        )
    return body_view(body, entries, working_headers)


class LineChunks:
    def __init__(self, lines: list[str]) -> None:
        self.lines = iter(lines)

    def __iter__(self) -> "LineChunks":
        return self

    def __next__(self) -> str:
        return next(self.lines)


def stream_answer(input_value: StreamInput) -> Json:
    source = STREAM_ITERATOR(
        streaming_response=LineChunks(input_value["lines"]),
        sync_stream=False,
        json_mode=input_value.get("json_mode"),
    )
    source.response_id = "chatcmpl-" + input_value["id"]
    chunks: list[Json] = [chunk_view(item) for item in source]
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
