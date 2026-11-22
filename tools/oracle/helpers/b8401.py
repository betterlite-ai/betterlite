from collections.abc import Callable, Mapping
from typing import Final, Literal, TypeAlias, TypedDict, cast

from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import ProxyLogging
from litellm.types.mcp import (
    MCPDuringCallRequestObject,
    MCPDuringCallResponseObject,
    MCPPreCallRequestObject,
    MCPPreCallResponseObject,
)
from pydantic import JsonValue, TypeAdapter

LlmResult: TypeAlias = ValueError | str | dict[str, JsonValue]

TO_LLM: Final = cast(Callable[[ProxyLogging, MCPPreCallRequestObject, Mapping[str, object]], Mapping[str, object]], ProxyLogging._convert_mcp_to_llm_format)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L1461-L1532
PRE_CALL: Final = cast(Callable[[ProxyLogging, LlmResult, MCPPreCallRequestObject], MCPPreCallResponseObject | None], ProxyLogging._convert_llm_result_to_mcp_response)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L1534-L1658
DURING_CALL: Final = cast(Callable[[ProxyLogging, LlmResult, MCPDuringCallRequestObject], MCPDuringCallResponseObject | None], ProxyLogging._convert_llm_result_to_mcp_during_response)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L1660-L1695

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)


class ToolRequest(TypedDict):
    tool_name: str
    arguments: dict[str, JsonValue]


class Outcome(TypedDict, total=False):
    error: str
    text: str
    data: dict[str, JsonValue]


class ToLlmScenario(TypedDict):
    function: Literal["to_llm"]
    request: ToolRequest
    kwargs: dict[str, JsonValue]
    premium: bool


class PreCallScenario(TypedDict):
    function: Literal["pre_call"]
    request: ToolRequest
    result: Outcome


class DuringCallScenario(TypedDict):
    function: Literal["during_call"]
    request: ToolRequest
    result: Outcome


Scenario: TypeAlias = ToLlmScenario | PreCallScenario | DuringCallScenario


class Call(TypedDict):
    scenarios: list[Scenario]


def run(input_value: Call) -> JsonValue:
    proxy = ProxyLogging(user_api_key_cache=UserApiKeyCache())
    return [scenario_answer(proxy, scenario) for scenario in input_value["scenarios"]]


def scenario_answer(proxy: ProxyLogging, scenario: Scenario) -> JsonValue:
    match scenario["function"]:
        case "to_llm":
            return to_llm_answer(proxy, scenario)
        case "pre_call" | "during_call":
            return hook_answer(proxy, scenario)


def to_llm_answer(proxy: ProxyLogging, scenario: ToLlmScenario) -> JsonValue:
    # the auth guardrails ask for an enterprise license, which each scenario sets for itself
    proxy_server.premium_user = scenario["premium"]
    request = MCPPreCallRequestObject(
        tool_name=scenario["request"]["tool_name"],
        arguments=scenario["request"]["arguments"],
    )
    kwargs: dict[str, JsonValue | UserAPIKeyAuth] = dict(scenario["kwargs"])
    auth = scenario["kwargs"].get("user_api_key_auth")
    if auth is not None:
        kwargs["user_api_key_auth"] = AUTH.validate_python(auth)
    return OBJECT.validate_python(TO_LLM(proxy, request, kwargs))


def hook_answer(proxy: ProxyLogging, scenario: PreCallScenario | DuringCallScenario) -> JsonValue:
    result = llm_result(scenario["result"])
    tool_name = scenario["request"]["tool_name"]
    arguments = scenario["request"]["arguments"]
    response: MCPPreCallResponseObject | MCPDuringCallResponseObject | None
    shown: set[str]
    match scenario["function"]:
        case "pre_call":
            pre_request = MCPPreCallRequestObject(tool_name=tool_name, arguments=arguments)
            response = PRE_CALL(proxy, result, pre_request)
            shown = {"should_proceed", "modified_arguments", "error_message"}
        case "during_call":
            during_request = MCPDuringCallRequestObject(tool_name=tool_name, arguments=arguments)
            response = DURING_CALL(proxy, result, during_request)
            shown = {"should_continue", "error_message"}
    if response is None:
        return None
    return OBJECT.validate_json(response.model_dump_json(include=shown))


def llm_result(outcome: Outcome) -> LlmResult:
    if "error" in outcome:
        return ValueError(outcome["error"])
    if "text" in outcome:
        return outcome["text"]
    if "data" in outcome:
        return outcome["data"]
    raise ValueError("a result names error, text or data")
