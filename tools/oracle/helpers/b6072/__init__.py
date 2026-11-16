import litellm
from pydantic import JsonValue

from .auth import header_value, key_value
from .chat import request_value
from .chunks import chunk_value
from .models import model_value
from .replies import reply_value
from .views import pin_clock
from .wire import Input, Object


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
