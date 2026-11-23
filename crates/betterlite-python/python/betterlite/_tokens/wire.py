from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite.types.utils import Message

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


# LiteLLM's convert_list_message_to_dict dumps a Message model; a plain dict passes through
def message_json(message: dict[str, JsonValue] | Message) -> JsonValue:
    match message:
        case Message():
            return message.model_dump()
        case dict():
            return message
