from typing import cast

import litellm
from litellm.llms.ollama.completion.transformation import OllamaConfig

from .support import Json, RequestCase, RequestInput, config_builder, dict_or_empty


def request_case(config: OllamaConfig, case: RequestCase) -> Json:
    headers: dict[str, Json] = {}
    try:
        body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L391
            "dict[str, Json]",
            config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L391
                model=case["model"],
                messages=case["messages"],  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L391
                optional_params=dict_or_empty(case.get("options")),
                litellm_params=dict_or_empty(case.get("litellm_params")),
                headers=headers,
            ),
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    return {"ok": True, "body": body}


def request_cases(input_value: RequestInput) -> Json:
    litellm.custom_prompt_dict = input_value["custom_prompt_dict"]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L399
    config = config_builder({})
    bodies: list[Json] = [request_case(config, case) for case in input_value["requests"]]
    return {"bodies": bodies}
