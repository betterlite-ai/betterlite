from typing import Literal

from pydantic import JsonValue, PrivateAttr
from typing_extensions import Required, TypedDict

from betterlite.types.utils.base import ResponseObject


class CreateVideoRequest(TypedDict, total=False):
    prompt: Required[str]
    input_reference: str | None
    model: str | None
    seconds: str | None
    size: str | None
    characters: list[dict[str, str]] | None
    user: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    timeout: float | None


class OpenAIVideoObject(ResponseObject):
    id: str
    object: Literal["video"]
    status: str
    created_at: int
    completed_at: int | None = None
    expires_at: int | None = None
    error: dict[str, JsonValue] | None = None
    progress: int | None = None
    remixed_from_video_id: str | None = None
    seconds: str | None = None
    size: str | None = None
    model: str | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
