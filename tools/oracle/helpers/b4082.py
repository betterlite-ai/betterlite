import json
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.integrations import shadow_eval_logger
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

OPS, CHAT_OPS, ANTHROPIC_OPS, RESPONSES_OPS, JUDGEABLE, POST_GUARDRAIL, HAS_HOSTED, SNAPSHOT = (shadow_eval_logger._SURFACE_OPS, shadow_eval_logger._CHAT_OPS, shadow_eval_logger._ANTHROPIC_OPS, shadow_eval_logger._RESPONSES_OPS, shadow_eval_logger._judgeable_sample, shadow_eval_logger._post_guardrail_kwargs, shadow_eval_logger._request_has_hosted_web_search, shadow_eval_logger.GuardrailRequestSnapshot)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/shadow_eval_logger.py#L93-L450

# betterlite's typed request omits unset optional fields, so null and absent are one value here
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
MESSAGE_LIST: Final = TypeAdapter(list[dict[str, JsonValue]])


class SurfaceOpsCall(TypedDict):
    operation: Literal["surface_ops"]
    call_type: str


class ChatRequestCall(TypedDict):
    operation: Literal["chat_request"]
    call_type: str
    kwargs: dict[str, JsonValue]
    model_parameters: dict[str, JsonValue]


class SampleCall(TypedDict):
    operation: Literal["sample"]
    call_type: str
    kwargs: dict[str, JsonValue]
    model_parameters: dict[str, JsonValue]
    response: dict[str, JsonValue]


class PostGuardrailCall(TypedDict):
    operation: Literal["post_guardrail_kwargs"]
    call_type: str
    kwargs: dict[str, JsonValue]
    metadata: dict[str, JsonValue]
    snapshot_body: dict[str, JsonValue]
    snapshot_metadata: dict[str, JsonValue]


class HostedWebSearchCall(TypedDict):
    operation: Literal["hosted_web_search"]
    request: dict[str, JsonValue]


Input: TypeAlias = (
    SurfaceOpsCall | ChatRequestCall | SampleCall | PostGuardrailCall | HostedWebSearchCall
)


def surface_row(call: SurfaceOpsCall) -> JsonValue:
    ops = OPS.get(call["call_type"])
    if ops is None:
        return {"found": False, "wire_params": None, "surface": None}
    if ops is CHAT_OPS:
        surface = "chat"
    elif ops is ANTHROPIC_OPS:
        surface = "anthropic_messages"
    else:
        surface = "responses"
    return {"found": True, "wire_params": ops.wire_params, "surface": surface}


def chat_request_row(call: ChatRequestCall) -> JsonValue:
    request = OPS[call["call_type"]].chat_request(call["kwargs"], call["model_parameters"])
    shaped = {name: value for name, value in request.items() if value is not None}
    return OBJECT.validate_json(json.dumps(shaped, default=dict))


def sample_of(call: SampleCall) -> JsonValue:
    sample = JUDGEABLE(
        OPS[call["call_type"]], call["kwargs"], call["model_parameters"], call["response"]
    )
    if sample is None:
        return None
    messages, params, real_text = sample
    return {
        "messages": MESSAGE_LIST.validate_json(json.dumps(messages, default=dict)),
        "params": OBJECT.validate_json(json.dumps(dict(params), default=dict)),
        "real_text": real_text,
    }


def guardrail_row(call: PostGuardrailCall) -> JsonValue:
    snapshot = SNAPSHOT.capture(call["snapshot_body"], call["snapshot_metadata"])
    rebuilt = POST_GUARDRAIL(call["kwargs"], call["metadata"], OPS[call["call_type"]], snapshot)
    if rebuilt is None:
        return None
    return OBJECT.validate_json(json.dumps(dict(rebuilt), default=dict))


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "surface_ops":
            return surface_row(input_value)
        case "chat_request":
            return chat_request_row(input_value)
        case "sample":
            return {"sample": sample_of(input_value)}
        case "post_guardrail_kwargs":
            return {"result": guardrail_row(input_value)}
        case "hosted_web_search":
            return {"has": HAS_HOSTED(input_value["request"])}
