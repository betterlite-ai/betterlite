from litellm.llms.databricks.chat.transformation import DatabricksConfig
from pydantic import JsonValue, TypeAdapter

from .common import Input, Object, ParamsCase, cases_of, chat_api, guarded

CHAT = chat_api(DatabricksConfig())


def params_of(case: ParamsCase) -> Object:
    optional: Object = dict(case["optional_params"])
    mapped = CHAT.map_openai_params(
        case["non_default_params"], optional, case["model"], drop_params=True
    )
    return {"params": mapped}


def params_value(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[ParamsCase]).validate_python(cases_of(input_value))
    return [guarded(params_of, case) for case in cases]
