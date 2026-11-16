from litellm.llms.databricks.chat.transformation import DatabricksConfig
from pydantic import JsonValue, TypeAdapter

from .common import JSON_ADAPTER, Input, Object, StreamCase, cases_of, chat_api, guarded

CHAT = chat_api(DatabricksConfig())


def chunks_of(case: StreamCase) -> Object:
    iterator = CHAT.get_model_response_iterator(
        None, sync_stream=False, json_mode=case["json_mode"]
    )
    chunks = [
        JSON_ADAPTER.validate_json(
            iterator.chunk_parser(chunk).model_dump_json(exclude_unset=True)
        )
        for chunk in case["chunks"]
    ]
    return {"chunks": chunks}


def stream_value(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[StreamCase]).validate_python(cases_of(input_value))
    return [guarded(chunks_of, case) for case in cases]
