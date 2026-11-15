from typing import Literal

from typing_extensions import TypedDict


class XAIWebSearchFilters(TypedDict, total=False):
    allowed_domains: list[str] | None
    excluded_domains: list[str] | None


class XAIWebSearchTool(TypedDict, total=False):
    type: Literal["web_search"]
    filters: XAIWebSearchFilters | None
    enable_image_understanding: bool | None


class XAIXSearchTool(TypedDict, total=False):
    type: Literal["x_search"]
    allowed_x_handles: list[str] | None
    excluded_x_handles: list[str] | None
    from_date: str | None
    to_date: str | None
    enable_image_understanding: bool | None
    enable_video_understanding: bool | None
