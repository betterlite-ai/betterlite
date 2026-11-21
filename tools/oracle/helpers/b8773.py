import asyncio
from typing import Final, Literal, NotRequired, TypedDict

from litellm.exceptions import ModifyResponseException
from litellm.proxy.guardrails.guardrail_hooks.litellm_content_filter.content_filter import (
    ContentFilterGuardrail,
)
from litellm.types.guardrails import BlockedWord, ContentFilterPattern, GuardrailEventHooks
from litellm.types.proxy.guardrails.guardrail_hooks.litellm_content_filter import (
    LitellmContentFilterGuardrailConfigModel as ConfigModel,
)
from litellm.types.utils import ChatCompletionMessageToolCall, Function
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

GUARDRAIL_NAME: Final = "content-filter"


class ToolCallInput(TypedDict):
    id: str
    type: Literal["function"]
    function: dict[str, str]


class CompetitorConfig(TypedDict):
    competitor_intent_type: Literal["generic", "airline"]
    brand_self: list[str]
    competitors: list[str]
    locations: list[str]
    policy: dict[str, str]
    threshold_high: float
    threshold_medium: float
    threshold_low: float
    refuse_message_template: str | None
    reframe_message_template: str | None


class ArgsInput(TypedDict):
    patterns: list[ContentFilterPattern]
    blocked_words: list[BlockedWord]
    event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | None
    default_on: bool
    guardrail_id: str | None
    policy_template: str | None
    image_model: str | None
    competitor_intent_config: CompetitorConfig | None
    severity_threshold: str


class ApplyInput(TypedDict):
    texts: list[str]
    tool_calls: list[ToolCallInput] | None
    input_type: Literal["request", "response"]


class BuildInput(TypedDict):
    args: ArgsInput
    apply: ApplyInput | None


class Input(TypedDict):
    builds: list[BuildInput]


class Inputs(TypedDict):
    texts: list[str]
    tool_calls: NotRequired[list[ChatCompletionMessageToolCall]]


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
TOOL_CALL: Final[TypeAdapter[ToolCallInput]] = TypeAdapter(ToolCallInput)
JSON: Final[TypeAdapter[Json]] = TypeAdapter(Json)


def new_guardrail(args: ArgsInput) -> ContentFilterGuardrail:
    config = args["competitor_intent_config"]
    return ContentFilterGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        guardrail_id=args["guardrail_id"],
        policy_template=args["policy_template"],
        patterns=args["patterns"],
        blocked_words=args["blocked_words"],
        event_hook=args["event_hook"],
        default_on=args["default_on"],
        severity_threshold=args["severity_threshold"],
        image_model=args["image_model"],
        competitor_intent_config=(
            None
            if config is None
            else {
                "competitor_intent_type": config["competitor_intent_type"],
                "brand_self": config["brand_self"],
                "competitors": config["competitors"],
                "locations": config["locations"],
                "policy": config["policy"],
                "threshold_high": config["threshold_high"],
                "threshold_medium": config["threshold_medium"],
                "threshold_low": config["threshold_low"],
                "refuse_message_template": config["refuse_message_template"],
                "reframe_message_template": config["reframe_message_template"],
            }
        ),
    )


# The fields a caller reads back from the constructed class, as LiteLLM stores them
def build_tag(guardrail: ContentFilterGuardrail) -> Json:
    patterns: list[Json] = [
        {"name": entry["pattern_name"], "action": JSON.validate_python(entry["action"])}
        for entry in guardrail.compiled_patterns
    ]
    blocked_words: dict[str, Json] = {
        keyword: [JSON.validate_python(action), description]
        for keyword, (action, description) in guardrail.blocked_words.items()
    }
    hooks: list[Json] = [
        JSON.validate_python(hook) for hook in type(guardrail).get_supported_event_hooks()
    ]
    return {
        "guardrail_provider": guardrail.guardrail_provider,
        "config_guardrail_id": guardrail.config_guardrail_id,
        "config_policy_template": guardrail.config_policy_template,
        "pattern_redaction_format": guardrail.pattern_redaction_format,
        "keyword_redaction_tag": guardrail.keyword_redaction_tag,
        "severity_threshold": guardrail.severity_threshold,
        "event_hook": JSON.validate_python(guardrail.event_hook),
        "default_on": guardrail.default_on,
        "image_model": guardrail.image_model,
        "patterns": patterns,
        "blocked_words": blocked_words,
        "supported_event_hooks": hooks,
        "config_model": ConfigModel.__name__,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/litellm_content_filter/content_filter.py#L2155-L2161
    }


def tool_call(item: ToolCallInput) -> ChatCompletionMessageToolCall:
    function = Function(arguments=item["function"]["arguments"], name=item["function"]["name"])
    return ChatCompletionMessageToolCall(function=function, id=item["id"], type=item["type"])


def parsed_arguments(arguments: str) -> Json:
    try:
        return JSON.validate_json(arguments)
    except ValueError:
        return arguments


def tool_tag(call: ChatCompletionMessageToolCall) -> Json:
    dump = TOOL_CALL.validate_json(call.model_dump_json())
    return {
        "name": dump["function"]["name"],
        "arguments": parsed_arguments(dump["function"]["arguments"]),
    }


async def apply_output(guardrail: ContentFilterGuardrail, apply_input: ApplyInput) -> Json:
    calls: list[ChatCompletionMessageToolCall] = []
    inputs: Inputs = {"texts": list(apply_input["texts"])}
    tool_calls = apply_input["tool_calls"]
    if tool_calls is not None:
        calls = [tool_call(item) for item in tool_calls]
        inputs["tool_calls"] = calls
    request_data: dict[str, Json] = {}
    try:
        await guardrail.apply_guardrail(inputs, request_data, apply_input["input_type"])  # type: ignore[arg-type, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536-L4547
    except StarletteHTTPException as raised:
        return {
            "texts": None,
            "tool_calls": None,
            "error": {
                "kind": "http",
                "status": raised.status_code,
                "detail": JSON.validate_python(raised.detail),
            },
        }
    except ModifyResponseException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1201
        return {
            "texts": None,
            "tool_calls": None,
            "error": {"kind": "modify_response", "message": raised.message},
        }
    return {
        "texts": list[Json](inputs["texts"]),
        "tool_calls": [tool_tag(call) for call in calls] if tool_calls is not None else None,
        "error": None,
    }


async def build_output(build_input: BuildInput) -> Json:
    guardrail = new_guardrail(build_input["args"])
    apply_input = build_input["apply"]
    applied: Json = None
    if apply_input is not None:
        applied = await apply_output(guardrail, apply_input)
    return {"build": build_tag(guardrail), "apply": applied}


async def run_builds(request: Input) -> Json:
    builds: list[Json] = [await build_output(build_input) for build_input in request["builds"]]
    return {"builds": builds}


def run(input_value: dict[str, Json]) -> Json:
    request = INPUT.validate_python(input_value)
    return asyncio.run(run_builds(request))
