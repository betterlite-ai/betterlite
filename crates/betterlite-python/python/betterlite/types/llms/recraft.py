from pydantic import JsonValue
from typing_extensions import TypedDict


# ast-grep-ignore: no-long-names
class RecraftImageGenerationRequestParams(TypedDict, total=False):
    prompt: str
    text_layout: list[dict[str, JsonValue]] | None
    n: int | None
    style_id: str | None
    style: str | None
    substyle: str | None
    model: str | None
    response_format: str | None
    size: str | None
    negative_prompt: str | None
    controls: dict[str, JsonValue] | None


class RecraftImageEditRequestParams(TypedDict, total=False):
    prompt: str
    strength: float
    model: str | None
    n: int | None
    style_id: str | None
    style: str | None
    substyle: str | None
    response_format: str | None
    negative_prompt: str | None
    controls: dict[str, JsonValue] | None
