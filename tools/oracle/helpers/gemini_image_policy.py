from typing import Final, TypeAlias, TypedDict

import litellm
from litellm.llms.gemini.common_utils import (
    get_gemini_image_generation_config,
    get_gemini_image_web_search_requests,
    is_gemini_image_model,
    map_gemini_image_tools_params,
    map_openai_image_params_to_gemini,
    supports_gemini_image_size,
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject: TypeAlias = dict[str, JsonValue]
_ADAPTER: Final = TypeAdapter(dict[str, JsonValue])


class Case(TypedDict):
    model: str
    catalog_supports_image_size: bool | None
    params: JsonObject
    supported_params: list[str]
    optional_params: JsonObject
    parse_image_config_string: bool
    pre_mapped: JsonObject | None
    response: JsonObject


class Input(TypedDict):
    settings: dict[str, str]
    cases: list[Case]


def case_result(case: Case) -> JsonValue:
    catalog = case["catalog_supports_image_size"]
    if catalog is not None:
        litellm.model_cost[case["model"]] = {"supports_image_size": catalog}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    mapped = _ADAPTER.validate_python(
        map_openai_image_params_to_gemini(
            params=case["params"],
            model=case["model"],
            supported_params=case["supported_params"],
            optional_params=case["optional_params"],
            parse_image_config_string=case["parse_image_config_string"],
        )
    )
    if case["pre_mapped"] is not None:
        mapped.update(case["pre_mapped"])
    tools = _ADAPTER.validate_python(map_gemini_image_tools_params(case["params"], mapped))
    return {
        "supports_image_size": supports_gemini_image_size(case["model"]),
        "is_image_model": is_gemini_image_model(case["model"]),
        "mapped": mapped,
        "tools": tools,
        "generation": _ADAPTER.validate_python(
            get_gemini_image_generation_config(case["model"], tools)
        ),
        "web_search_requests": get_gemini_image_web_search_requests(case["response"]),
    }


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = [case_result(case) for case in input_value["cases"]]
    return {"results": results}
