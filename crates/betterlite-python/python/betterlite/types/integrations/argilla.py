from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict


class ArgillaItem(TypedDict):
    fields: dict[str, JsonValue]


class ArgillaPayload(TypedDict):
    items: list[ArgillaItem]


class ArgillaCredentialsObject(TypedDict):
    ARGILLA_API_KEY: str
    ARGILLA_DATASET_NAME: str
    ARGILLA_BASE_URL: str


SUPPORTED_PAYLOAD_FIELDS: Final = ["messages", "response"]
