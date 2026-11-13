import litellm
from litellm.llms.openai.image_variations.transformation import OpenAIImageVariationConfig
from pydantic import JsonValue as Json

from .probe import DOCUMENT, Probe, data_view, file_object

litellm.suppress_debug_info = True


def empty_answer(supported: list[Json]) -> dict[str, Json]:
    return {
        "supported": supported,
        "mapped": None,
        "data": None,
        "supplied": None,
        "kind": None,
        "calls": [],
        "result": None,
        "error": None,
        "pre_call": None,
        "post_call": None,
    }


def supported_value(config: OpenAIImageVariationConfig, model: str) -> list[Json]:
    found: list[Json] = []
    for name in config.get_supported_openai_params(model=model):
        found.append(name)
    return found


def transform_value(probe: Probe) -> Json:
    config = OpenAIImageVariationConfig()
    answer = empty_answer(supported_value(config, probe.model or ""))
    data = config.transform_request_image_variation(
        model=probe.model,
        image=file_object(probe),
        optional_params=dict(probe.optional_params),
        headers={},
    )
    match data.get("data"):
        case dict() as fields:
            answer["data"] = data_view(DOCUMENT.validate_python(fields), probe)
        case None | bool() | int() | float() | str() | list():
            answer["data"] = None
    return answer


def mapped_value(probe: Probe) -> Json:
    config = OpenAIImageVariationConfig()
    answer = empty_answer(supported_value(config, probe.model or ""))
    optional_params: dict[str, Json] = dict(probe.existing_params)
    config.map_openai_params(
        non_default_params=dict(probe.non_default_params),
        optional_params=optional_params,
        model=probe.model or "",
        drop_params=False,
    )
    answer["mapped"] = optional_params
    return answer
