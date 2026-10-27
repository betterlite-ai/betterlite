from typing import TypeAlias, cast

from litellm.integrations.websearch_interception.tools import (
    get_litellm_web_search_tool,
    get_litellm_web_search_tool_openai,
    get_litellm_web_search_tool_responses,
    is_web_search_tool,
    is_web_search_tool_chat_completion,
    is_web_search_tool_responses,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = cast(str, input_value["function"])
    tool = cast(dict[str, object], input_value.get("tool", {}))
    return dispatch(function, input_value, tool)


def dispatch(function: str, input_value: dict[str, Json], tool: dict[str, object]) -> Json:
    if function.startswith("get_litellm"):
        return builders(function)
    if function.startswith("is_web_search"):
        return detection(function, tool)
    if function == "search_outcome_succeeded":
        return {"text": input_value.get("text", ""), "response": input_value.get("response")}
    if function == "search_outcome_failed":
        return {
            "error_code": input_value.get("error_code"),
            "message": input_value.get("message", ""),
        }
    if function == "search_outcome_error_codes":
        return cast(Json, error_codes())
    raise ValueError(f"unknown websearch function: {function}")


def builders(function: str) -> Json:
    if function == "get_litellm_web_search_tool":
        return cast(Json, get_litellm_web_search_tool())
    if function == "get_litellm_web_search_tool_openai":
        return cast(Json, get_litellm_web_search_tool_openai())
    return cast(Json, get_litellm_web_search_tool_responses())


def detection(function: str, tool: dict[str, object]) -> bool:
    if function == "is_web_search_tool_responses":
        return is_web_search_tool_responses(tool)
    if function == "is_web_search_tool_chat_completion":
        return is_web_search_tool_chat_completion(tool)
    return is_web_search_tool(tool)


def error_codes() -> list[str]:
    return [
        "invalid_tool_input",
        "unavailable",
        "max_uses_exceeded",
        "too_many_requests",
        "query_too_long",
        "request_too_large",
    ]
