import io
import os
import pathlib
from collections.abc import Callable
from typing import Final, Protocol, TypeAlias, runtime_checkable

import httpx
from litellm.llms.gemini.image_edit import get_gemini_image_edit_config
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

from studio_image_edit.edit_values import JsonObject

ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False
STUDIO_VARIABLES: Final = ("GEMINI_API_KEY", "GEMINI_API_BASE", "GOOGLE_API_KEY")
# the edit response transform ignores its model argument, so one literal stands in for every case
EDIT_MODEL: Final[str] = "gemini-2.5-flash-image"

# the file records LiteLLM's edit reader handles, and one it refuses
Stored: TypeAlias = bytes | io.BytesIO | pathlib.Path
ImageInput: TypeAlias = Stored | list[Stored | None] | None


@runtime_checkable
class GeminiEditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    use_multipart_form_data: Callable[[], bool]
    map_openai_params: Callable[[dict[str, JsonValue], str, bool], dict[str, JsonValue]]
    get_complete_url: Callable[[str, str | None, dict[str, JsonValue]], str]
    validate_environment: Callable[
        [dict[str, str], str, str | None, dict[str, JsonValue], str | None],
        dict[str, JsonValue],
    ]
    transform_image_edit_request: Callable[
        [
            str,
            str | None,
            ImageInput,
            dict[str, JsonValue],
            dict[str, JsonValue],
            dict[str, str],
        ],
        tuple[dict[str, JsonValue], list[JsonValue]],
    ]
    transform_image_edit_response: Callable[[str, httpx.Response, None], ImageResponse]


# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[GeminiEditConfig] = TypeAdapter[GeminiEditConfig](
    GeminiEditConfig, config=ARBITRARY_TYPES
).validate_python(get_gemini_image_edit_config(EDIT_MODEL))


def apply_environment(env: dict[str, str] | None) -> None:
    # only the three Studio variables change, so dispatch's cost-map setting stays in place
    for name in STUDIO_VARIABLES:
        os.environ.pop(name, None)
    if env is not None:
        os.environ.update(env)


def factory_value(model: str) -> JsonObject:
    config = get_gemini_image_edit_config(model)
    return {"model": model, "config": type(config).__name__}
