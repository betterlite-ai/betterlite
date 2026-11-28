import asyncio
from collections.abc import Mapping, Sequence
from typing import Final, Literal, NotRequired, TypedDict, cast

import fastuuid
import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.responses.mcp import litellm_proxy_mcp_handler as handler
from litellm.responses.mcp.litellm_proxy_mcp_handler import LiteLLM_Proxy_MCP_Handler as Proxy
from litellm.responses.mcp.litellm_proxy_mcp_handler import MCPToolResult
from litellm.responses.mcp.mcp_streaming_iterator import MCPEnhancedStreamingIterator
from litellm.types.llms.openai import ResponsesAPIResponse, ResponsesAPIStreamingResponse
from openai.types.responses.tool_param import ToolParam as ResponsesToolParam
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

FORWARDED: Final = ("input", "model", "tools", "previous_response_id")
ToolList = Sequence[ResponsesToolParam] | None
EventList = list[ResponsesAPIStreamingResponse]


class RequestSpec(TypedDict):
    input: str
    model: str
    tools: list[JsonValue] | None
    previous_response_id: str | None


class _AuthNone(TypedDict):
    kind: Literal["none"]


class _AuthDirect(TypedDict):
    kind: Literal["direct"]
    user_id: str


class _AuthMetadata(TypedDict):
    kind: Literal["metadata"]
    user_id: str | None


_AuthSpec = _AuthNone | _AuthDirect | _AuthMetadata


class RequestScenario(TypedDict):
    request: RequestSpec
    call_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    auth: NotRequired[_AuthSpec]
    mcp_tools: list[dict[str, JsonValue]]
    mcp_events: list[dict[str, JsonValue]]
    tool_server_map: dict[str, str]


class RequestParamsCase(TypedDict):
    operation: Literal["request_params"]
    scenarios: list[RequestScenario]


class ToolExecutionEventsCase(TypedDict):
    operation: Literal["tool_execution_events"]
    tool_calls: list[dict[str, JsonValue]]
    tool_results: list[MCPToolResult]
    ids: list[str]


class CallParameterStreamingCase(TypedDict):
    operation: Literal["call_parameter_streaming"]
    call_params: dict[str, JsonValue]
    should_auto_execute: bool
    original_stream: bool


class AppendOutputCase(TypedDict):
    operation: Literal["append_output"]
    response: dict[str, JsonValue]
    mcp_tools_fetched: list[JsonValue]
    tool_results: list[MCPToolResult]
    ids: list[str]


class ProviderCallErrorCase(TypedDict):
    operation: Literal["provider_call_error"]
    request: RequestSpec
    call_params: dict[str, JsonValue]
    follow_up_input: list[JsonValue]
    status: int


CaseInput = (
    RequestParamsCase
    | ToolExecutionEventsCase
    | CallParameterStreamingCase
    | AppendOutputCase
    | ProviderCallErrorCase
)

JSON: Final = TypeAdapter(JsonValue)


class _FixedUuid:
    def __init__(self, ids: list[str]) -> None:
        self._ids = list(ids)

    def __call__(self) -> fastuuid.UUID:
        return fastuuid.UUID(hex=self._ids.pop(0))


class _ProviderError(Exception):
    def __init__(self, status: int) -> None:
        super().__init__("the follow-up responses call failed")
        self.status_code = status


class _Recorder:
    def __init__(self, status: int) -> None:
        self.calls: list[dict[str, JsonValue]] = []
        self._status = status

    async def __call__(self, **kwargs: JsonValue) -> ResponsesAPIResponse:
        self.calls.append(kwargs)
        raise _ProviderError(self._status)


def _auth_state(
    scenario: RequestScenario,
) -> tuple[UserAPIKeyAuth | None, dict[str, UserAPIKeyAuth | None]]:
    auth = scenario.get("auth")
    if auth is None:
        return None, {}
    if auth["kind"] == "direct":
        return UserAPIKeyAuth.model_validate({"user_id": auth["user_id"]}), {}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356
    principal = None if auth["user_id"] is None else UserAPIKeyAuth.model_validate({"user_id": auth["user_id"]})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356
    return None, {"user_api_key_auth": principal}


def _streaming_iterator(scenario: RequestScenario) -> MCPEnhancedStreamingIterator:
    request = scenario["request"]
    direct, metadata = _auth_state(scenario)
    return cast(MCPEnhancedStreamingIterator, Proxy._create_mcp_streaming_response(  # noqa: SLF001  # pyright: ignore[reportAny, reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1145-L1188
        input=request["input"],
        model=request["model"],
        all_tools=request["tools"],
        mcp_tools_with_litellm_proxy=cast(list[Mapping[str, object]], scenario["mcp_tools"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1145-L1188
        mcp_discovery_events=cast(EventList, scenario["mcp_events"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1145-L1188
        call_params=scenario["call_params"],
        previous_response_id=request["previous_response_id"],
        tool_server_map=scenario["tool_server_map"],
        user_api_key_auth=direct,
        litellm_metadata=metadata,
        **scenario["kwargs"],
    ))


def _request_scenario(scenario: RequestScenario) -> JsonValue:
    iterator = _streaming_iterator(scenario)
    params: dict[str, JsonValue] = iterator.original_request_params  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/mcp_streaming_iterator.py#L288
    del params["user_api_key_auth"]
    del params["litellm_metadata"]
    principal = iterator.user_api_key_auth
    view: dict[str, JsonValue] = {
        "request_params": params,
        "auth_user_id": None if principal is None else principal.user_id,
        "mcp_events": JSON.validate_python(iterator.mcp_events),
        "mcp_tools": JSON.validate_python(scenario["mcp_tools"]),
        "tool_server_map": JSON.validate_python(iterator.tool_server_map),
    }
    return JSON.validate_python(view)


def _tool_execution_events(value: ToolExecutionEventsCase) -> JsonValue:
    fastuuid.uuid4 = _FixedUuid(value["ids"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1267
    events = Proxy._create_tool_execution_events(tool_calls=value["tool_calls"], tool_results=value["tool_results"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1224-L1272
    dumps: list[JsonValue] = [cast(JsonValue, event.model_dump(mode="json")) for event in events]  # pyright: ignore[reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1889
    return JSON.validate_python({"events": dumps})


def _call_parameter_streaming(value: CallParameterStreamingCase) -> JsonValue:
    initial: dict[str, JsonValue] = Proxy._prepare_initial_call_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1274-L1288
        value["call_params"],
        value["should_auto_execute"],
    )
    follow_up: dict[str, JsonValue] = Proxy._prepare_follow_up_call_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1290-L1308
        value["call_params"],
        value["original_stream"],
    )
    view: dict[str, JsonValue] = {"initial_params": initial, "follow_up_params": follow_up}
    return JSON.validate_python(view)


def _append_output(value: AppendOutputCase) -> JsonValue:
    response = ResponsesAPIResponse.model_validate(value["response"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1388
    fastuuid.uuid4 = _FixedUuid(value["ids"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1326
    composed = Proxy._add_mcp_output_elements_to_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1310-L1357
        response,
        value["mcp_tools_fetched"],
        value["tool_results"],
    )
    return JSON.validate_python(cast(dict[str, JsonValue], composed.model_dump(mode="json"))["output"])  # pyright: ignore[reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1354-L1355


def _provider_call_error(value: ProviderCallErrorCase) -> JsonValue:
    request = value["request"]
    call_params: dict[str, JsonValue] = Proxy._prepare_follow_up_call_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1290-L1308
        call_params=value["call_params"],
        original_stream_setting=False,
    )
    recorder = _Recorder(value["status"])
    handler.aresponses = recorder  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L19
    try:
        _outcome = asyncio.run(
            Proxy._make_follow_up_call(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/mcp/litellm_proxy_mcp_handler.py#L1103-L1118
                follow_up_input=value["follow_up_input"],
                model=request["model"],
                all_tools=cast(ToolList, request["tools"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L61
                response_id=request["previous_response_id"],
                **call_params,
            )
        )
    except _ProviderError as error:
        recorded = recorder.calls[0]
        forwarded: dict[str, JsonValue] = {
            "input": recorded["input"],
            "model": recorded["model"],
            "tools": recorded["tools"],
            "previous_response_id": recorded["previous_response_id"],
            "call_params": {
                name: item for name, item in recorded.items() if name not in FORWARDED
            },
        }
        status_view: dict[str, JsonValue] = {"status": error.status_code}
        view: dict[str, JsonValue] = {"request": forwarded, "error": status_view}
        return JSON.validate_python(view)
    raise ValueError("the provider call must fail")


def run(input_value: CaseInput) -> JsonValue:
    if input_value["operation"] == "request_params":
        answers: list[JsonValue] = [_request_scenario(item) for item in input_value["scenarios"]]
        return JSON.validate_python(answers)
    if input_value["operation"] == "tool_execution_events":
        return _tool_execution_events(input_value)
    if input_value["operation"] == "call_parameter_streaming":
        return _call_parameter_streaming(input_value)
    if input_value["operation"] == "append_output":
        return _append_output(input_value)
    return _provider_call_error(input_value)
