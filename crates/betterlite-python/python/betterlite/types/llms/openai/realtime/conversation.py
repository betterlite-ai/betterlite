from typing import Literal

from pydantic import JsonValue
from typing_extensions import Required, TypedDict

from betterlite.types.llms.openai.realtime.items import (
    OpenAIRealtimeStreamResponseOutputItem,
)


class OpenAIRealtimeStreamResponseBaseObject(TypedDict):  # ast-grep-ignore: no-long-names
    event_id: str
    response: dict[str, JsonValue]
    type: str


class OpenAIRealtimeConversationObject(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    id: str
    object: Required[Literal["realtime.conversation"]]


class OpenAIRealtimeConversationCreated(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    type: Required[Literal["conversation.created"]]
    conversation: OpenAIRealtimeConversationObject  # ast-grep-ignore: no-long-names
    event_id: str


# ast-grep-ignore: no-long-names
class OpenAIRealtimeConversationItemCreated(TypedDict, total=False):
    type: Required[Literal["conversation.item.created"]]
    item: OpenAIRealtimeStreamResponseOutputItem  # ast-grep-ignore: no-long-names
    event_id: str
    previous_item_id: str | None


class OpenAIRealtimeConversationItemAdded(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    type: Required[Literal["conversation.item.added"]]
    item: OpenAIRealtimeStreamResponseOutputItem  # ast-grep-ignore: no-long-names
    event_id: str
    previous_item_id: str | None


class OpenAIRealtimeConversationItemDone(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    type: Required[Literal["conversation.item.done"]]
    item: OpenAIRealtimeStreamResponseOutputItem  # ast-grep-ignore: no-long-names
    event_id: str
    previous_item_id: str | None
