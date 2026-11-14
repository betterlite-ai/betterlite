from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict

POSTHOG_MAX_BATCH_SIZE: Final = 100


class PostHogEventPayload(TypedDict):
    event: str
    properties: dict[str, JsonValue]
    distinct_id: str


class PostHogCredentialsObject(TypedDict):
    POSTHOG_API_KEY: str
    POSTHOG_HOST: str
