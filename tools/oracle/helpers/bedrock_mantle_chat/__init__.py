from pydantic import JsonValue

from .inputs import Input
from .responses import messages_response, native_sse
from .transforms import chat, messages_request
from .urls import urls


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "chat_url" | "message_url":
            return urls(input_value)
        case "messages_request":
            return messages_request(input_value)
        case "chat_request":
            return chat(input_value, inline=False)
        case "remote_url_inlined":
            return chat(input_value, inline=True)
        case "native_sse":
            return native_sse(input_value)
        case "messages_response":
            return messages_response(input_value)
