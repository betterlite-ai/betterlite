from collections.abc import Callable
from typing import cast

from litellm.llms.ollama.completion.transformation import OllamaConfig

from .support import Json, ParamCase, ParamsInput, config_builder, dict_or_empty


def mapped_case(config: OllamaConfig, case: ParamCase) -> Json:
    optional = dict_or_empty(case.get("optional"))
    try:
        options = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L195
            "dict[str, Json]",
            config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L195
                non_default_params=case["non_default"],
                optional_params=optional,
                model=case["model"],
                drop_params=False,
            ),
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    return {"ok": True, "options": options}


def param_cases(input_value: ParamsInput) -> Json:
    config = config_builder(input_value["config"])
    mapped: list[Json] = [mapped_case(config, case) for case in input_value["cases"]]
    empty: dict[str, Json] = {}
    urls: list[Json] = [
        config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L455
            api_base=url,
            api_key=None,
            model="model",
            optional_params=empty,
            litellm_params=empty,
        )
        for url in input_value["urls"]
    ]
    supported = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L181
        "list[Json]", config.get_supported_openai_params(model="model")
    )
    required = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L166
        "Json", config.get_required_params()
    )
    shown = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L167
        "Callable[[], dict[str, Json]]", config.get_config
    )()
    return {
        "supported": supported,
        "required": required,
        "config": shown,
        "mapped": mapped,
        "urls": urls,
    }
