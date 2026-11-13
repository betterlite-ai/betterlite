from typing import Final, Literal, NotRequired, TypedDict

from litellm.proxy._types import ProxyException
from litellm.proxy.openai_files_endpoints.common_utils import (
    convert_b64_uid_to_unified_uid,
    decode_model_from_file_id,
    encode_batch_response_ids,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L232
    encode_file_id_with_model,
    get_batch_id_from_unified_batch_id,
    get_model_id_from_unified_batch_id,
    get_models_from_unified_file_id,
    get_original_file_id,
    is_litellm_executed_batch,
    is_model_embedded_id,
    resolve_managed_output_file_model_name,
    validate_file_list_limit,
)
from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class LimitCall(TypedDict):
    function: Literal["validate_limit"]
    limit: int | None


class ConvertCall(TypedDict):
    function: Literal["convert"]
    value: str


class TargetModelsCall(TypedDict):
    function: Literal["target_models"]
    value: str | int | None


class ResolveCall(TypedDict):
    function: Literal["resolve_output_model"]
    value: str | None
    fallback: str | None


class ModelIdCall(TypedDict):
    function: Literal["model_id"]
    value: str | int | None


class BatchIdCall(TypedDict):
    function: Literal["batch_id"]
    value: str | int | None


class ExecutedCall(TypedDict):
    function: Literal["executed"]
    value: str


class EncodeCall(TypedDict):
    function: Literal["encode"]
    value: str
    model: str
    id_type: Literal["file", "batch"]


class DecodeModelCall(TypedDict):
    function: Literal["decode_model"]
    value: str


class OriginalIdCall(TypedDict):
    function: Literal["original_id"]
    value: str


class ModelEmbeddedCall(TypedDict):
    function: Literal["model_embedded"]
    value: str


class BatchResponse(TypedDict):
    id: str
    completion_window: str
    created_at: int
    endpoint: str
    input_file_id: str
    object: Literal["batch"]
    status: str
    output_file_id: NotRequired[str | None]
    error_file_id: NotRequired[str | None]


class BatchCall(TypedDict):
    function: Literal["encode_batch_response"]
    response: BatchResponse
    model: str


ReaderCall = (
    ConvertCall
    | TargetModelsCall
    | ModelIdCall
    | BatchIdCall
    | ExecutedCall
    | DecodeModelCall
    | OriginalIdCall
    | ModelEmbeddedCall
)

Call = LimitCall | ResolveCall | EncodeCall | BatchCall | ReaderCall


class Input(TypedDict):
    calls: list[Call]


def limit_answer(limit: int | None) -> JsonValue:
    try:
        validate_file_list_limit(limit)
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {
            "accepted": False,
            "message": error.message,
            "type": error.type,
            "param": error.param,
            "code": error.code,
            "openai_code": error.openai_code,
            "headers": JSON.validate_python(error.headers),
        }
    return {"accepted": True}


def batch_answer(response: BatchResponse, model: str) -> JsonValue:
    batch = LiteLLMBatch.model_validate(response)
    encode_batch_response_ids(batch, model)
    return JSON.validate_json(batch.model_dump_json())


def target_models_answer(value: str | int | None) -> JsonValue:
    # the reader guards with isinstance(str), and cases send other JSON values on purpose
    guarded: str = value  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L144
    return JSON.validate_python(get_models_from_unified_file_id(guarded))


def model_id_answer(value: str | int | None) -> JsonValue:
    guarded: str = value  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L164
    return get_model_id_from_unified_batch_id(guarded)


def batch_id_answer(value: str | int | None) -> JsonValue:
    guarded: str = value  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L174
    try:
        batch_id = get_batch_id_from_unified_batch_id(guarded)
    except IndexError:
        return {"raised": True}
    return batch_id


def encoded_answer(call: DecodeModelCall | OriginalIdCall | ModelEmbeddedCall) -> JsonValue:
    if call["function"] == "decode_model":
        return decode_model_from_file_id(call["value"])
    if call["function"] == "original_id":
        return get_original_file_id(call["value"])
    return is_model_embedded_id(call["value"])


def reader_answer(call: ReaderCall) -> JsonValue:
    if call["function"] == "convert":
        return convert_b64_uid_to_unified_uid(call["value"])
    if call["function"] == "target_models":
        return target_models_answer(call["value"])
    if call["function"] == "model_id":
        return model_id_answer(call["value"])
    if call["function"] == "batch_id":
        return batch_id_answer(call["value"])
    if call["function"] == "executed":
        return is_litellm_executed_batch(call["value"])
    return encoded_answer(call)


def answer(call: Call) -> JsonValue:
    if call["function"] == "validate_limit":
        return limit_answer(call["limit"])
    if call["function"] == "encode_batch_response":
        return batch_answer(call["response"], call["model"])
    if call["function"] == "encode":
        return encode_file_id_with_model(
            file_id=call["value"], model=call["model"], id_type=call["id_type"]
        )
    if call["function"] == "resolve_output_model":
        return resolve_managed_output_file_model_name(call["value"], call["fallback"])
    return reader_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return answers
