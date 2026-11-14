from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.openai.core.aliases import FileTypes


class ImageEditOptionalRequestParams(TypedDict, total=False):
    background: Literal["transparent", "opaque", "auto"] | None
    input_fidelity: Literal["high", "low"] | None
    mask: str | None
    n: int | None
    quality: Literal["high", "medium", "low", "standard", "auto"] | None
    response_format: Literal["url", "b64_json"] | None
    size: str | None
    user: str | None
    imageConfig: dict[str, JsonValue] | None


class ImageEditRequestParams(ImageEditOptionalRequestParams, total=False):
    image: FileTypes
    prompt: str
    model: str | None
