from typing import Final, Literal, TypedDict

from litellm.llms.base_llm.files.batch_records import responses_batch_body_to_chat_body
from litellm.llms.base_llm.files.litellm_db_storage_backend import storage_url_to_row_id
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class StorageRowIdCall(TypedDict):
    function: Literal["storage_row_id"]
    storage_url: str


class ResponsesBatchCall(TypedDict):
    function: Literal["responses_batch"]
    body: dict[str, JsonValue]
    custom_llm_provider: str | None


Call = StorageRowIdCall | ResponsesBatchCall


class Input(TypedDict):
    calls: list[Call]


def row_id_answer(call: StorageRowIdCall) -> JsonValue:
    try:
        row_id = storage_url_to_row_id(call["storage_url"])
    except ValueError as error:
        return {"message": str(error)}
    return {"row_id": row_id}


def batch_answer(call: ResponsesBatchCall) -> JsonValue:
    try:
        body = responses_batch_body_to_chat_body(
            call["body"],
            custom_llm_provider=call["custom_llm_provider"],
        )
    except ValueError as error:
        return {"message": str(error)}
    return JSON.validate_python(body)


def answer(call: Call) -> JsonValue:
    if call["function"] == "storage_row_id":
        return row_id_answer(call)
    return batch_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
