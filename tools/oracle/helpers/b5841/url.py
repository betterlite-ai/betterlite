from litellm.llms.databricks.chat.transformation import DatabricksConfig
from pydantic import JsonValue, TypeAdapter

from .common import ARBITRARY, ChatApi, Input, Object, UrlCase, cases_of, guarded

CHAT = TypeAdapter(ChatApi, config=ARBITRARY).validate_python(DatabricksConfig())


def url_of(case: UrlCase) -> Object:
    url = CHAT.get_complete_url(case["api_base"], None, case["model"], {}, {})
    return {"url": url}


def url_value(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[UrlCase]).validate_python(cases_of(input_value))
    return [guarded(url_of, case) for case in cases]
