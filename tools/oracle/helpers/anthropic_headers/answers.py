from collections.abc import Mapping
from typing import Final

import httpx
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .contracts import (
    CREATE_MODEL_LIST,
    GET_HEADERS,
    PROCESS_HEADERS,
    VALIDATE,
    Call,
    EnvironmentCall,
    HeadersCall,
    ModelListCall,
    ResponseHeadersCall,
)

BETA_HEADER: Final = "anthropic-beta"
LIST_ADAPTER: Final = TypeAdapter(dict[str, Json])


def project_headers(headers: Mapping[str, str]) -> dict[str, Json]:
    projected: dict[str, Json] = {}
    for name, value in headers.items():
        if name == BETA_HEADER:
            projected[name] = sorted(set(value.split(",")))
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
    validated = VALIDATE(
        headers,
        call["model"],
        call["messages"],
        call["optional_params"],
        call["litellm_params"],
        api_key=call["api_key"],
        api_base=call["api_base"],
    )
    return {"headers": project_headers(validated)}


def model_list_answer(call: ModelListCall) -> Json:
    response = CREATE_MODEL_LIST(
        call["models"], call["display_names"], call["listed_ids"]
    )
    return {"response": LIST_ADAPTER.validate_python(response)}


def response_headers_answer(call: ResponseHeadersCall) -> Json:
    headers = httpx.Headers(
        [(header["name"], header["value"]) for header in call["headers"]]
    )
    return {"headers": project_headers(PROCESS_HEADERS(headers))}


def answer(call: Call) -> Json:
    match call["function"]:
        case "headers":
            return headers_answer(call)
        case "environment":
            return environment_answer(call)
        case "model_list":
            return model_list_answer(call)
        case "response_headers":
            return response_headers_answer(call)
