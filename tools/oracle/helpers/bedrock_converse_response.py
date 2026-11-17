import datetime as dt
import time
from functools import partial
from typing import TypedDict, cast

import fastuuid
import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.llms.bedrock.common_utils import BedrockError, error_response_text
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

CONFIG = AmazonConverseConfig()
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL_RESPONSE: TypeAdapter[ModelResponse] = TypeAdapter(ModelResponse)
EMPTY: dict[str, JsonValue] = {}
MODEL = "fixture-model"
litellm.suppress_debug_info = True


class SuppliedFields(TypedDict):
    id: str
    model: str
    object: str
    system_fingerprint: str | None
    # null keeps the member unset, so the dump leaves it out
    service_tier: JsonValue
    trace: JsonValue


class TransformCase(TypedDict):
    body: str
    headers: list[tuple[str, str]]
    model: str
    messages: JsonValue
    data: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    api_key: str | None
    json_mode: bool | None
    supplied: SuppliedFields
    ids: list[str]
    now: int


class FakeStreamCase(TypedDict):
    model: str | None
    stream: bool | None
    provider: str | None
    fake_stream: bool | None


class EnvironmentCase(TypedDict):
    fields: list[str]
    headers: list[tuple[str, str]]
    litellm_params: dict[str, JsonValue]
    api_key: str | None


class ErrorClassCase(TypedDict):
    message: str
    status: int
    headers: list[tuple[str, str]]


class Input(TypedDict, total=False):
    transform: list[TransformCase]
    fake_stream: list[FakeStreamCase]
    environment: list[EnvironmentCase]
    error_class: list[ErrorClassCase]


class Details(BaseModel):
    input: JsonValue = None
    original_response: JsonValue = None
    additional_args: dict[str, JsonValue] | None = None
    log_event_type: str | None = None


class Recorded(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: Details


class FixedUuids:
    def __init__(self, values: list[str]) -> None:
        self.values = list(values)

    def __call__(self) -> fastuuid.UUID:
        return fastuuid.UUID(hex=self.values.pop(0))


def _logging() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )


def _log_view(log: Logging) -> JsonValue:
    details = Recorded.model_validate(log).model_call_details
    return {
        "input": details.input,
        "original_response": details.original_response,
        "additional_args": details.additional_args,
        "log_event_type": details.log_event_type,
    }


def _supplied(fields: SuppliedFields) -> ModelResponse:
    supplied: dict[str, JsonValue] = {
        "id": fields["id"],
        "created": 0,
        "model": fields["model"],
        "object": fields["object"],
        "choices": [],
        "usage": None,
        "system_fingerprint": fields["system_fingerprint"],
    }
    if fields["service_tier"] is not None:
        supplied["service_tier"] = fields["service_tier"]
    if fields["trace"] is not None:
        supplied["trace"] = fields["trace"]
    return MODEL_RESPONSE.validate_python(supplied)


def _pairs(headers: httpx.Headers) -> list[JsonValue]:
    encoding = headers.encoding
    return [[name.decode(encoding), value.decode(encoding)] for name, value in headers.raw]


# A response built around the case body: httpx would add a Content-Length the case did not send.
def _reply(case: TransformCase) -> httpx.Response:
    response = httpx.Response(200, headers=case["headers"], content=case["body"].encode())
    if not any(name.lower() == "content-length" for name, _ in case["headers"]):
        del response.headers["content-length"]
    return response


def _transform(case: TransformCase) -> JsonValue:
    response = _reply(case)
    log = _logging()
    outcome: JsonValue = None
    fastuuid.uuid4 = FixedUuids(case["ids"])
    time.time = partial(float, case["now"])
    try:
        result = CONFIG.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L2079-L2105
            model=case["model"],
            raw_response=response,
            model_response=_supplied(case["supplied"]),
            logging_obj=log,
            request_data=case["data"],
            messages=cast(list[AllMessageValues], case["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
            optional_params=case["optional_params"],
            litellm_params=EMPTY,
            encoding=None,
            api_key=case["api_key"],
            json_mode=case["json_mode"],
        )
    except BedrockError as error:
        headers = cast(httpx.Headers, error.headers)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/common_utils.py#L78-L102
        outcome = {
            "failure": {"status": error.status_code, "headers": _pairs(headers)},
            "log": _log_view(log),
        }
    except (AttributeError, KeyError, TypeError, ValueError):
        outcome = {"failure": {"status": None, "headers": None}, "log": _log_view(log)}
    else:
        outcome = {
            "response": JSON.validate_json(result.model_dump_json()),
            "log": _log_view(log),
        }
    return outcome


def _fake_stream(case: FakeStreamCase) -> JsonValue:
    return CONFIG.should_fake_stream(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L2726-L2752
        model=case["model"],
        stream=case["stream"],
        custom_llm_provider=case["provider"],
        fake_stream=case["fake_stream"],
    )


def _environment(case: EnvironmentCase) -> JsonValue:
    litellm.bedrock_request_metadata_fields = case["fields"]
    caller: dict[str, str] = dict(case["headers"])
    headers = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L2711-L2724
        "dict[str, str]",
        CONFIG.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L2711-L2724
            headers=caller,
            model=MODEL,
            messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L2711-L2724
            optional_params=EMPTY,
            litellm_params=case["litellm_params"],
            api_key=case["api_key"],
            api_base=None,
        ),
    )
    pairs: list[JsonValue] = [[name, value] for name, value in headers.items()]
    return {"headers": pairs}


def _error_class(case: ErrorClassCase) -> JsonValue:
    supplied = httpx.Headers(case["headers"])
    error = CONFIG.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L2704-L2709
        case["message"], case["status"], supplied
    )
    return _error_view(cast(BedrockError, error))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/common_utils.py#L78-L102


def _error_view(error: BedrockError) -> JsonValue:
    headers = cast(httpx.Headers, error.headers)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/common_utils.py#L78-L102
    response = error.response
    return {
        "status_code": error.status_code,
        "status_code_is_synthesized": error.status_code_is_synthesized,
        "message": error.message,
        "headers": _pairs(headers),
        "request": {"method": error.request.method, "url": str(error.request.url)},
        "response": {
            "status_code": response.status_code,
            "text": error_response_text(response),
        },
    }


def run(input_value: Input) -> JsonValue:
    if "transform" in input_value:
        return [_transform(case) for case in input_value["transform"]]
    if "fake_stream" in input_value:
        return [_fake_stream(case) for case in input_value["fake_stream"]]
    if "environment" in input_value:
        return [_environment(case) for case in input_value["environment"]]
    if "error_class" in input_value:
        return [_error_class(case) for case in input_value["error_class"]]
    raise ValueError("unknown bedrock_converse_response operation")
