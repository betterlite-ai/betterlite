import litellm
from pydantic import JsonValue

from .chunks import chunk_value
from .models import model_value
from .replies import reply_value
from .views import CHAT, pin_clock
from .wire import MODEL, HeaderCall, Input, KeyCall, Object, RequestCall, activate


def header_value(call: HeaderCall) -> JsonValue:
    activate(call["environment"])
    return CHAT.validate_environment(
        call["headers"], MODEL, [], {}, call["litellm_params"], call["api_key"], None
    )


def key_value(call: KeyCall) -> JsonValue:
    activate(call["environment"])
    return CHAT.get_api_key(api_key=call["api_key"])


def request_value(call: RequestCall) -> JsonValue:
    return CHAT.transform_request(call["model"], call["messages"], call["optional_params"], {}, {})


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = {entry["key"]: entry["model_info"] for entry in input_value["catalog"]}
    pin_clock()
    result: Object = {
        "headers": [header_value(call) for call in input_value["headers"]],
        "keys": [key_value(call) for call in input_value["keys"]],
        "requests": [request_value(call) for call in input_value["requests"]],
        "replies": [reply_value(call) for call in input_value["replies"]],
        "chunks": [chunk_value(call) for call in input_value["chunks"]],
        "models": [model_value(call) for call in input_value["models"]],
    }
    return result
