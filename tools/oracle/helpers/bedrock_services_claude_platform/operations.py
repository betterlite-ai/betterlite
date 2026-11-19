from typing import Literal

from pydantic import JsonValue as Json

from .types import (
    AUTHENTICATION_ERROR,
    CHAT_TRANSFORM,
    CHAT_VALIDATE,
    CHUNK_VIEW,
    EMPTY_MESSAGES,
    EMPTY_PARAMS,
    EMPTY_STRINGS,
    NATIVE_TRANSFORM,
    NATIVE_VALIDATE,
    STREAM_ITERATOR,
    BodyInput,
    HasDump,
    HeaderInput,
    Input,
    StreamInput,
    WorkspaceFields,
    WorkspaceInput,
)
from .views import (
    apply_environment,
    body_view,
    captured_warnings,
    failure_view,
    merged_params,
    params_of,
    project_headers,
    workspace_view,
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
