import asyncio
from typing import Final, Literal, TypedDict

from litellm.exceptions import ModifyResponseException
from litellm.proxy.guardrails.guardrail_hooks.litellm_content_filter.content_filter import (
    ContentFilterGuardrail,
)
from litellm.router import Router
from litellm.types.guardrails import BlockedWord, ContentFilterPattern, GuardrailEventHooks
from litellm.types.utils import (
    ChatCompletionMessageToolCall,
    Function,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

GUARDRAIL_NAME: Final = "content-filter-inputs"


class Dispatch(TypedDict):
    operation: Literal["images", "competitor", "tool_calls", "mcp"]


class ImagesInput(TypedDict):
    patterns: list[ContentFilterPattern]
    blocked_words: list[BlockedWord]
    images: list[str]
    image_model: str
    image_description: str


class CompetitorConfig(TypedDict):
    brand_self: list[str]
    competitors: list[str]
    locations: list[str]
    policy: dict[str, str]
    threshold_high: float
    threshold_medium: float
    threshold_low: float
    refuse_message_template: str | None
    reframe_message_template: str | None


class CompetitorInput(TypedDict):
    text: str
    model: str
    config: CompetitorConfig
    patterns: list[ContentFilterPattern]
    blocked_words: list[BlockedWord]


class ToolCallInput(TypedDict):
    id: str
    type: str
    function: dict[str, str]


class ToolCallsInput(TypedDict):
    patterns: list[ContentFilterPattern]
    blocked_words: list[BlockedWord]
    tool_calls: list[ToolCallInput]


class McpInput(TypedDict):
    patterns: list[ContentFilterPattern]
    blocked_words: list[BlockedWord]
    request_data: dict[str, Json]


class Inputs(TypedDict, total=False):
    texts: list[str]
    images: list[str]
    tool_calls: list[ChatCompletionMessageToolCall]


DISPATCH: Final[TypeAdapter[Dispatch]] = TypeAdapter(Dispatch)
IMAGES_INPUT: Final[TypeAdapter[ImagesInput]] = TypeAdapter(ImagesInput)
COMPETITOR_INPUT: Final[TypeAdapter[CompetitorInput]] = TypeAdapter(CompetitorInput)
TOOL_CALLS_INPUT: Final[TypeAdapter[ToolCallsInput]] = TypeAdapter(ToolCallsInput)
MCP_INPUT: Final[TypeAdapter[McpInput]] = TypeAdapter(McpInput)
TOOL_CALL_DUMP: Final[TypeAdapter[ToolCallInput]] = TypeAdapter(ToolCallInput)
JSON: Final[TypeAdapter[Json]] = TypeAdapter(Json)


def detection_tag(value: Json) -> Json | None:
    match value:
        case {"type": "pattern", "pattern_name": str() as name}:
            return {"type": "pattern", "pattern_name": name}
        case {"type": str() as kind}:
            return {"type": kind}
        case None | bool() | int() | float() | str() | list() | dict():
            return None


def detection_tags(response: Json) -> list[Json]:
    match response:
        case list() as items:
            return [tag for item in items if (tag := detection_tag(item)) is not None]
        case None | bool() | int() | float() | str() | dict():
            return []


def record_of(request_data: dict[str, Json]) -> dict[str, Json]:
    match request_data.get("metadata"):
        case {"standard_logging_guardrail_information": [dict() as record, *_]}:
            return dict[str, Json](record)
        case None | bool() | int() | float() | str() | list() | dict():
            return {}


def output(
    texts: list[Json],
    tool_calls: list[Json],
    request_data: dict[str, Json],
    error: Json,
) -> Json:
    return {
        "texts": texts,
        "detections": detection_tags(record_of(request_data).get("guardrail_response")),
        "tool_calls": tool_calls,
        "request_data": {
            "mcp_arguments": request_data.get("mcp_arguments"),
            "modified_arguments": request_data.get("modified_arguments"),
        },
        "error": error,
    }


def http_error(raised: StarletteHTTPException) -> Json:
    detail: Json = JSON.validate_python(raised.detail)
    return {"kind": "http", "status": raised.status_code, "detail": detail}


def passthrough_error(raised: ModifyResponseException) -> Json:
    return {
        "kind": "passthrough",
        "message": raised.message,
        "model": raised.model,
        "guardrail_name": raised.guardrail_name,
    }


async def guardrail_error(
    guardrail: ContentFilterGuardrail,
    inputs: Inputs,
    request_data: dict[str, Json],
) -> Json:
    try:
        await guardrail.apply_guardrail(inputs, request_data, "request")  # type: ignore[arg-type, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536-L4547
    except StarletteHTTPException as raised:
        return http_error(raised)
    except ModifyResponseException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1201-L1233
        return passthrough_error(raised)
    return None


def tool_call(item: ToolCallInput) -> ChatCompletionMessageToolCall:
    function = Function(
        arguments=item["function"]["arguments"], name=item["function"]["name"]
    )
    return ChatCompletionMessageToolCall(function=function, id=item["id"], type=item["type"])


def tool_call_tag(call: ChatCompletionMessageToolCall) -> Json:
    dump = TOOL_CALL_DUMP.validate_json(call.model_dump_json())
    try:
        arguments: Json = JSON.validate_json(dump["function"]["arguments"])
    except ValueError:
        arguments = dump["function"]["arguments"]
    function: Json = {"name": dump["function"]["name"], "arguments": arguments}
    return {"id": dump["id"], "type": dump["type"], "function": function}


def filtered_texts(inputs: Inputs) -> list[Json]:
    texts = inputs.get("texts")
    if texts is None:
        return []
    return list[Json](texts)


async def run_images(request: ImagesInput) -> Json:
    params: dict[str, Json] = {
        "model": "openai/gpt-4o-mini",
        "mock_response": request["image_description"],
    }
    router = Router(model_list=[{"model_name": "vision-model", "litellm_params": params}])
    guardrail = ContentFilterGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        patterns=request["patterns"],
        blocked_words=request["blocked_words"],
        llm_router=router,
        image_model=request["image_model"],
    )
    inputs: Inputs = {"texts": [], "images": list(request["images"])}
    request_data: dict[str, Json] = {}
    error = await guardrail_error(guardrail, inputs, request_data)
    return output([], [], request_data, error)


async def run_competitor(request: CompetitorInput) -> Json:
    guardrail = ContentFilterGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        patterns=request["patterns"],
        blocked_words=request["blocked_words"],
        event_hook=GuardrailEventHooks.pre_call,
        competitor_intent_config={
            "competitor_intent_type": "airline",
            "brand_self": request["config"]["brand_self"],
            "competitors": request["config"]["competitors"],
            "locations": request["config"]["locations"],
            "policy": request["config"]["policy"],
            "threshold_high": request["config"]["threshold_high"],
            "threshold_medium": request["config"]["threshold_medium"],
            "threshold_low": request["config"]["threshold_low"],
            "refuse_message_template": request["config"]["refuse_message_template"],
            "reframe_message_template": request["config"]["reframe_message_template"],
        },
    )
    inputs: Inputs = {"texts": [request["text"]]}
    request_data: dict[str, Json] = {"model": request["model"]}
    error = await guardrail_error(guardrail, inputs, request_data)
    return output(filtered_texts(inputs), [], request_data, error)


async def run_tool_calls(request: ToolCallsInput) -> Json:
    calls = [tool_call(item) for item in request["tool_calls"]]
    guardrail = ContentFilterGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        patterns=request["patterns"],
        blocked_words=request["blocked_words"],
        event_hook=GuardrailEventHooks.pre_call,
    )
    inputs: Inputs = {"texts": [], "tool_calls": calls}
    request_data: dict[str, Json] = {}
    error = await guardrail_error(guardrail, inputs, request_data)
    projected: list[Json] = [tool_call_tag(call) for call in calls]
    return output(filtered_texts(inputs), projected, request_data, error)


async def run_mcp(request: McpInput) -> Json:
    guardrail = ContentFilterGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        patterns=request["patterns"],
        blocked_words=request["blocked_words"],
        event_hook=GuardrailEventHooks.pre_mcp_call,
    )
    inputs: Inputs = {"texts": []}
    request_data: dict[str, Json] = dict(request["request_data"])
    error = await guardrail_error(guardrail, inputs, request_data)
    return output(filtered_texts(inputs), [], request_data, error)


def run(input_value: dict[str, Json]) -> Json:
    operation = DISPATCH.validate_python(input_value)["operation"]
    if operation == "images":
        return asyncio.run(run_images(IMAGES_INPUT.validate_python(input_value)))
    if operation == "competitor":
        return asyncio.run(run_competitor(COMPETITOR_INPUT.validate_python(input_value)))
    if operation == "tool_calls":
        return asyncio.run(run_tool_calls(TOOL_CALLS_INPUT.validate_python(input_value)))
    if operation == "mcp":
        return asyncio.run(run_mcp(MCP_INPUT.validate_python(input_value)))
    raise ValueError(f"unknown operation: {operation}")
