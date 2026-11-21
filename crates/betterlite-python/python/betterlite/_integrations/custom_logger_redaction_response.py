import copy

from pydantic import JsonValue

from betterlite._integrations.custom_logger_types import REDACTED_CONTENT
from betterlite.types.utils import Choices, Message, ModelResponse


def redacted_output_item(output_item: JsonValue) -> JsonValue:
    match output_item:
        case dict():
            output_payload: dict[str, JsonValue] = output_item
            content_value = output_payload.get("content")
            match content_value:
                case list():
                    content_items: list[JsonValue] = content_value
                    for content_item in content_items:
                        match content_item:
                            case dict():
                                content_payload: dict[str, JsonValue] = content_item
                                if "text" in content_payload:
                                    content_payload["text"] = REDACTED_CONTENT
                            case str() | bool() | int() | float() | list() | None:
                                pass
                case str() | bool() | int() | float() | dict() | None:
                    pass
        case str() | bool() | int() | float() | list() | None:
            pass
    return output_item


def redacted_response(response: JsonValue) -> JsonValue:
    match response:
        case dict():
            response_payload: dict[str, JsonValue] = response
            if "output" not in response_payload:
                return ModelResponse(
                    choices=[Choices(message=Message(content=REDACTED_CONTENT))]
                ).model_dump()
            response_copy = copy.deepcopy(response_payload)
            output_value = response_copy.get("output")
            match output_value:
                case list():
                    output_items: list[JsonValue] = output_value
                    response_copy["output"] = [
                        redacted_output_item(output_item) for output_item in output_items
                    ]
                case str() | bool() | int() | float() | dict() | None:
                    pass
            return response_copy
        case str() | bool() | int() | float() | list() | None:
            return ModelResponse(
                choices=[Choices(message=Message(content=REDACTED_CONTENT))]
            ).model_dump()
