import os
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import httpx
from litellm.llms.anthropic import common_utils
from litellm.llms.anthropic.common_utils import (
    AnthropicModelInfo,
    create_anthropic_model_list_response,
)
from litellm.types.proxy.model_listing import ModelInfoResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

BETA_HEADER: Final = "anthropic-beta"

BetaName: TypeAlias = Literal[
    "code_execution",
    "effort",
    "file_id",
    "input_examples",
    "mcp_server",
    "output_config",
    "pdf",
    "programmatic_tool_calling",
    "prompt_caching",
    "skills",
    "thinking_display_updates",
    "tool_changes",
    "tool_search",
]


class Header(TypedDict):
    name: str
    value: str


class HeadersArguments(TypedDict):
    api_key: NotRequired[str | None]
    auth_token: NotRequired[str | None]
    anthropic_version: NotRequired[str | None]
    computer_tool_used: NotRequired[str | None]
    prompt_caching_set: NotRequired[bool]
    pdf_used: NotRequired[bool]
    file_id_used: NotRequired[bool]
    mcp_server_used: NotRequired[bool]
    web_search_tool_used: NotRequired[bool]
    tool_search_used: NotRequired[bool]
    programmatic_tool_calling_used: NotRequired[bool]
    input_examples_used: NotRequired[bool]
    effort_used: NotRequired[bool]
    is_vertex_request: NotRequired[bool]
    user_anthropic_beta_headers: NotRequired[list[str] | None]
    code_execution_tool_used: NotRequired[bool]
    container_with_skills_used: NotRequired[bool]
    api_base: NotRequired[str | None]
    use_bearer_for_custom_base: NotRequired[bool]
    is_mid_conversation_output_config_used: NotRequired[bool]  # ast-grep-ignore: no-long-names
    is_thinking_display_updates_used: NotRequired[bool]  # ast-grep-ignore: no-long-names
    is_mid_conversation_tool_change_used: NotRequired[bool]  # ast-grep-ignore: no-long-names


class HeadersGetter(Protocol):
    def __call__(self, **kwargs: Unpack[HeadersArguments]) -> dict[str, str]: ...


class EnvironmentArguments(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class EnvironmentValidator(Protocol):
    def __call__(self, **kwargs: Unpack[EnvironmentArguments]) -> dict[str, str]: ...


class ResponseHeaderProjector(Protocol):
    def __call__(self, headers: Mapping[str, str]) -> dict[str, str]: ...


GET_HEADERS: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L951
    HeadersGetter, AnthropicModelInfo().get_anthropic_headers
)
VALIDATE_ENVIRONMENT: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L1048
    EnvironmentValidator, AnthropicModelInfo().validate_environment
)
PROCESS_HEADERS: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L1783
    ResponseHeaderProjector, common_utils.process_anthropic_headers
)


class HeadersCall(TypedDict):
    function: Literal["headers"]
    api_key: str | None
    auth_token: str | None
    anthropic_version: str | None
    api_base: str | None
    use_bearer_for_custom_base: bool
    user_betas: list[str] | None
    features: list[BetaName]
    computer_tool_used: str | None
    is_vertex_request: bool
    web_search_tool_used: bool


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    model: str
    headers: list[Header]
    api_key: str | None
    api_base: str | None
    litellm_params: dict[str, Json]
    messages: list[Json]
    optional_params: dict[str, Json]
    effort_used: bool


class ModelListCall(TypedDict):
    function: Literal["model_list"]
    models: list[ModelInfoResponse]
    display_names: dict[str, str]
    listed_ids: dict[str, str]


class ResponseHeadersCall(TypedDict):
    function: Literal["response_headers"]
    headers: list[Header]


Call: TypeAlias = HeadersCall | EnvironmentCall | ModelListCall | ResponseHeadersCall


class Input(TypedDict):
    environment: dict[str, str]
    calls: list[Call]


BETA_ADAPTER: Final = TypeAdapter(list[Json])
LIST_ADAPTER: Final = TypeAdapter(dict[str, Json])


def project_headers(headers: Mapping[str, str]) -> dict[str, Json]:
    projected: dict[str, Json] = {}
    for name, value in headers.items():
        if name == BETA_HEADER:
            tokens = sorted(set(value.split(",")))
            projected[name] = BETA_ADAPTER.validate_python(tokens)
        else:
            projected[name] = value
    return projected


def headers_answer(call: HeadersCall) -> Json:
    features = call["features"]
    headers = GET_HEADERS(
        api_key=call["api_key"],
        auth_token=call["auth_token"],
        anthropic_version=call["anthropic_version"],
        computer_tool_used=call["computer_tool_used"],
        prompt_caching_set="prompt_caching" in features,
        pdf_used="pdf" in features,
        file_id_used="file_id" in features,
        mcp_server_used="mcp_server" in features,
        web_search_tool_used=call["web_search_tool_used"],
        tool_search_used="tool_search" in features,
        programmatic_tool_calling_used="programmatic_tool_calling" in features,
        input_examples_used="input_examples" in features,
        effort_used="effort" in features,
        is_vertex_request=call["is_vertex_request"],
        user_anthropic_beta_headers=call["user_betas"],
        code_execution_tool_used="code_execution" in features,
        container_with_skills_used="skills" in features,
        api_base=call["api_base"],
        use_bearer_for_custom_base=call["use_bearer_for_custom_base"],
        is_mid_conversation_output_config_used="output_config" in features,
        is_thinking_display_updates_used="thinking_display_updates" in features,
        is_mid_conversation_tool_change_used="tool_changes" in features,
    )
    return {"headers": project_headers(headers)}


def environment_answer(call: EnvironmentCall) -> Json:
    headers: dict[str, str] = {
        header["name"]: header["value"] for header in call["headers"]
    }
    validated = VALIDATE_ENVIRONMENT(
        headers=headers,
        model=call["model"],
        messages=call["messages"],
        optional_params=call["optional_params"],
        litellm_params=call["litellm_params"],
        api_key=call["api_key"],
        api_base=call["api_base"],
    )
    return {"headers": project_headers(validated)}


def model_list_answer(call: ModelListCall) -> Json:
    response = create_anthropic_model_list_response(
        call["models"], call["display_names"], call["listed_ids"]
    )
    return {"response": LIST_ADAPTER.validate_python(response)}


def response_headers_answer(call: ResponseHeadersCall) -> Json:
    headers = httpx.Headers(
        [(header["name"], header["value"]) for header in call["headers"]]
    )
    return {"headers": project_headers(PROCESS_HEADERS(headers))}


def answer(call: Call) -> Json:
    if call["function"] == "headers":
        return headers_answer(call)
    if call["function"] == "environment":
        return environment_answer(call)
    if call["function"] == "model_list":
        return model_list_answer(call)
    return response_headers_answer(call)


def run(input_value: Input) -> Json:
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        return {"calls": [answer(call) for call in input_value["calls"]]}
    finally:
        os.environ.clear()
        os.environ.update(previous)
