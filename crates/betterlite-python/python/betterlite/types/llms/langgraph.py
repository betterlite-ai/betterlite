from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


class LangGraphMessage(TypedDict, total=False):
    role: Literal["human", "assistant", "system"]
    content: str


class LangGraphInput(TypedDict, total=False):
    messages: list[LangGraphMessage]


class LangGraphRequest(TypedDict, total=False):
    assistant_id: str
    input: LangGraphInput
    stream_mode: str | None
    config: dict[str, JsonValue] | None
    metadata: dict[str, JsonValue] | None


class LangGraphStreamEvent(TypedDict, total=False):
    event: str
    data: JsonValue


class LangGraphResponseMessage(TypedDict, total=False):
    type: str
    content: str
    id: str | None
    name: str | None


class LangGraphResponse(TypedDict, total=False):
    messages: list[LangGraphResponseMessage]
    values: dict[str, JsonValue]


class LangGraphParsedResponse(TypedDict):
    content: str
    role: str
    usage: dict[str, int] | None
