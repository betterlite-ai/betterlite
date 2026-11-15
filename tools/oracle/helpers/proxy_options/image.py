from typing import Literal, TypedDict

from pydantic import JsonValue, TypeAdapter

from .boundary import EDIT, GENERATION, VALUE
from .environment import install_environment


class ImageCase(TypedDict):
    endpoint: Literal["generations", "edits"]
    model: str
    api_base: str | None
    api_key: str | None
    environment_base: str | None
    environment_key: str | None
    headers: dict[str, str]


def image_transport(case: ImageCase) -> tuple[str, dict[str, str]]:
    headers: dict[str, str] = dict(case["headers"])
    if case["endpoint"] == "generations":
        url = GENERATION.get_complete_url(
            case["api_base"], case["api_key"], case["model"], {}, {}, None
        )
        return url, GENERATION.validate_environment(
            headers, case["model"], [], {}, {}, case["api_key"], case["api_base"]
        )
    url = EDIT.get_complete_url(case["model"], case["api_base"], {})
    return url, EDIT.validate_environment(
        headers, case["model"], case["api_key"], {}, case["api_base"]
    )


def image_value(case: ImageCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=None,
        openai_key=None,
    )
    try:
        url, headers = image_transport(case)
    except ValueError:
        return {"ok": False}
    return {"ok": True, "url": url, "headers": VALUE.validate_python(headers)}


def image_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ImageCase]).validate_python(params)
    return {"results": [image_value(case) for case in cases]}
