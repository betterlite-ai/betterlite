from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite.types.utils import Message

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


# LiteLLM dumps a Message without its None fields; a plain dict passes through
def message_json(message: dict[str, JsonValue] | Message) -> JsonValue:
    match message:
        case Message():
            return message.model_dump(exclude_none=True)
        case dict():
            return message
