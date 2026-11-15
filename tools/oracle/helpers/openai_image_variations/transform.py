import litellm
from pydantic import JsonValue as Json

from .config import variation_config
from .probe import Probe, data_view, file_object, text_of

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


def transform_value(probe: Probe) -> Json:
    model = text_of(probe.model)
    config = variation_config(model)
    names: list[Json] = list(config.get_supported_openai_params(model))
    answer = empty_answer(names)
    fields = config.transform_request_image_variation(
        probe.model,
        file_object(probe),
        dict(probe.optional_params),
        {},
    )
    answer["data"] = data_view(fields["data"], probe)
    return answer


def mapped_value(probe: Probe) -> Json:
    model = text_of(probe.model)
    config = variation_config(model)
    names: list[Json] = list(config.get_supported_openai_params(model))
    answer = empty_answer(names)
    optional_params: dict[str, Json] = dict(probe.existing_params)
    non_default_params: dict[str, Json] = dict(probe.non_default_params)
    config.map_openai_params(
        non_default_params=non_default_params,
        optional_params=optional_params,
        model=model,
        drop_params=False,
    )
    answer["mapped"] = optional_params
    return answer
