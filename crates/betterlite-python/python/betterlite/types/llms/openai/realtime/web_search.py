from typing import Literal

from typing_extensions import TypedDict


# ast-grep-ignore: no-long-names
class OpenAIWebSearchUserLocationApproximate(TypedDict):
    city: str
    country: str
    region: str
    timezone: str


class OpenAIWebSearchUserLocation(TypedDict):
    approximate: OpenAIWebSearchUserLocationApproximate  # ast-grep-ignore: no-long-names
    type: Literal["approximate"]


class OpenAIWebSearchOptions(TypedDict, total=False):
    search_context_size: Literal["low", "medium", "high"] | None
    user_location: OpenAIWebSearchUserLocation | None
