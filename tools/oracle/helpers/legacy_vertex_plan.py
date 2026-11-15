from collections.abc import Callable
from typing import Final, NotRequired, TypedDict, cast

from litellm.llms.vertex_ai import vertex_ai_non_gemini

CacheKey = Callable[[str, str | None, str | None], str]

CACHE_KEY: Final[CacheKey] = cast(CacheKey, vertex_ai_non_gemini._get_client_cache_key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAny, reportUnknownArgumentType, reportUnknownMemberType, reportUnnecessaryCast]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_ai_non_gemini.py#L58


class Input(TypedDict):
    model: str
    project: NotRequired[str | None]
    location: NotRequired[str | None]


def run(input_value: Input) -> str:
    model = input_value["model"]
    project = input_value.get("project")
    location = input_value.get("location")
    return CACHE_KEY(model, project, location)
