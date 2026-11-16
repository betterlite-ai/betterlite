import httpx
from litellm.llms.databricks.chat.transformation import DatabricksConfig
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .common import (
    ARBITRARY,
    JSON_ADAPTER,
    MODEL,
    ChatApi,
    Input,
    Object,
    ResponseCase,
    cases_of,
    guarded,
    logging_object,
)

CHAT = TypeAdapter(ChatApi, config=ARBITRARY).validate_python(DatabricksConfig())


def reply_of(case: ResponseCase) -> Object:
    response = CHAT.transform_response(
        MODEL,
        httpx.Response(200, json=case["body"]),
        ModelResponse(),
        logging_object(),
        {},
        [],
        {},
        {},
        None,
        None,
        json_mode=case["json_mode"],
    )
    return {"reply": JSON_ADAPTER.validate_json(response.model_dump_json(exclude_unset=True))}


def response_value(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[ResponseCase]).validate_python(cases_of(input_value))
    return [guarded(reply_of, case) for case in cases]
