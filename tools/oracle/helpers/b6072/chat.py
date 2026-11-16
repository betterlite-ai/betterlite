from pydantic import JsonValue

from .views import CHAT
from .wire import RequestCall


def request_value(call: RequestCall) -> JsonValue:
    return CHAT.transform_request(
        call["model"], call["messages"], call["optional_params"], {}, {}
    )
