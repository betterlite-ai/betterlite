import json

from pydantic import JsonValue
from typing_extensions import TypedDict, Unpack, override


class ServerSentEventFields(TypedDict, total=False):
    event: str | None
    data: str | None
    id: str | None
    retry: int | None


class ServerSentEvent:
    def __init__(self, **fields: Unpack[ServerSentEventFields]) -> None:
        for name in fields:
            if name not in ServerSentEventFields.__optional_keys__:
                raise TypeError(f"ServerSentEvent() got an unexpected keyword argument '{name}'")
        data = fields.get("data")
        event = fields.get("event")
        self._id = fields.get("id")
        self._data = "" if data is None else data
        self._event = None if event == "" else event
        self._retry = fields.get("retry")

    @property
    def event(self) -> str | None:
        return self._event

    @property
    def id(self) -> str | None:
        return self._id

    @property
    def retry(self) -> int | None:
        return self._retry

    @property
    def data(self) -> str:
        return self._data

    def json(self) -> JsonValue:
        parsed: JsonValue = json.loads(self.data)
        return parsed

    @override
    def __repr__(self) -> str:
        return (
            f"ServerSentEvent(event={self.event}, data={self.data}, "
            f"id={self.id}, retry={self.retry})"
        )
