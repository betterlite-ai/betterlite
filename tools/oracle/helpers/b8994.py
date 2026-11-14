from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.common_utils import callback_utils
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
REQUEST_DATA: Final = TypeAdapter(dict[str, JsonValue])


class RecordScanFunction(Protocol):
    def __call__(
        self,
        request_data: dict[str, JsonValue],
        scan_id: str | None,
        *,
        guardrail_name: str | None,
        provider: str,
        stage: GuardrailEventHooks,
    ) -> None: ...


class HeadersFunction(Protocol):
    def __call__(
        self, request_data: dict[str, JsonValue]
    ) -> dict[str, JsonValue] | None: ...


class AppendFunction(Protocol):
    def __call__(
        self,
        litellm_logging_obj: Logging | None,
        guardrail_response: dict[str, JsonValue],
    ) -> JsonValue: ...


@runtime_checkable
class CallbackUtils(Protocol):
    get_logging_caching_headers: HeadersFunction
    add_guardrail_scan_id: RecordScanFunction
    # ast-grep-ignore: no-long-names
    add_guardrail_response_to_standard_logging_object: AppendFunction


class CallbackUtilsRecord(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    utils: CallbackUtils


# LiteLLM leaves these helpers untyped, so this record names the signatures the cases call
CALLBACK_UTILS_VALUES: Final = {"utils": callback_utils}
CALLBACK_UTILS: Final = CallbackUtilsRecord.model_validate(CALLBACK_UTILS_VALUES).utils


class RecordScanInput(TypedDict):
    function: Literal["record_scan_id"]
    request_data: dict[str, JsonValue]
    scan_id: str | None
    guardrail_name: str | None
    provider: str
    stage: str


class HeadersInput(TypedDict):
    function: Literal["headers"]
    request_data: dict[str, JsonValue]


class AppendInput(TypedDict):
    function: Literal["append_standard_object"]
    model_call_details: dict[str, JsonValue] | None
    record: dict[str, JsonValue]


# The oracle sorts input keys, while add_guardrail_scan_id builds entries in this field order
class ScanEntryRecord(BaseModel):
    guardrail: str | None
    stage: str
    provider: str
    scan_id: str


class MetadataBucket(BaseModel):
    model_config = ConfigDict(extra="allow")
    guardrail_scan_metadata: list[ScanEntryRecord] | None = None


class RequestRecord(BaseModel):
    model_config = ConfigDict(extra="allow")
    metadata: MetadataBucket | None = None
    litellm_metadata: MetadataBucket | None = None


class LoggingRecord(Logging):
    model_call_details: dict[str, JsonValue]

    def __init__(self, model_call_details: dict[str, JsonValue]) -> None:
        self.model_call_details = dict(model_call_details)


def record_scan_answer(input_value: RecordScanInput) -> dict[str, JsonValue]:
    request_data = dict(input_value["request_data"])
    CALLBACK_UTILS.add_guardrail_scan_id(
        request_data=request_data,
        scan_id=input_value["scan_id"],
        guardrail_name=input_value["guardrail_name"],
        provider=input_value["provider"],
        stage=GuardrailEventHooks(input_value["stage"]),
    )
    return {"request_data": request_data}


def headers_answer(input_value: HeadersInput) -> JsonValue:
    request = RequestRecord.model_validate(input_value["request_data"])
    request_data = REQUEST_DATA.validate_json(
        request.model_dump_json(exclude_unset=True)
    )
    return JSON.validate_python(
        CALLBACK_UTILS.get_logging_caching_headers(request_data)
    )


def append_answer(input_value: AppendInput) -> dict[str, JsonValue]:
    details = input_value["model_call_details"]
    if details is None:
        CALLBACK_UTILS.add_guardrail_response_to_standard_logging_object(
            None, input_value["record"]
        )
        return {"model_call_details": None}
    logging_object = LoggingRecord(dict(details))
    CALLBACK_UTILS.add_guardrail_response_to_standard_logging_object(
        logging_object, input_value["record"]
    )
    return {
        "model_call_details": JSON.validate_python(logging_object.model_call_details)
    }


def run(input_value: RecordScanInput | HeadersInput | AppendInput) -> JsonValue:
    if input_value["function"] == "record_scan_id":
        return record_scan_answer(input_value)
    if input_value["function"] == "headers":
        return headers_answer(input_value)
    return append_answer(input_value)
