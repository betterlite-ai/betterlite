import os

os.environ["LITELLM_RUST"] = "false"

import time
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Final
from unittest.mock import MagicMock

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/boto/botocore/blob/879f8440a4e9ace5d3cf145ce8b3d5e5ffb892ef/botocore/auth.py
import httpx
import litellm
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.bedrock.batches.transformation import (
    BedrockBatchesConfig,
    titan_embedding_usage_from_batch_output,
)
from litellm.types.llms.openai import CreateBatchRequest
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
DEFAULT_UUID: Final = "00000000-0000-4000-8000-000000000000"
REAL_TIME: Final = time.time
# boto3's environment provider hands these to LiteLLM when a scenario names no credentials
TEST_KEY: Final = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"


@dataclass
class Credentials:
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None


@dataclass
class Scenario:
    model: str = ""
    input_file_id: str | None = None
    completion_window: str | None = None
    litellm_params: dict[str, JsonValue] = field(default_factory=dict)
    optional_params: dict[str, JsonValue] = field(default_factory=dict)
    environment: dict[str, str] = field(default_factory=dict)
    original_batch_request: JsonValue = None
    response_body: str = ""
    clock: list[int] = field(default_factory=list)
    job_prefix: str | None = None
    uuid: str = DEFAULT_UUID
    credentials: Credentials | None = None
    now: int = 0


@dataclass
class Input:
    operation: str
    scenarios: list[Scenario] = field(default_factory=list)
    values: list[JsonValue] = field(default_factory=list)


@dataclass
class SignedRequest:
    method: str
    url: str
    headers: dict[str, str]
    data: str


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = UUID(value)

    def __call__(self) -> UUID:
        return self.value


class FixedNow:
    def __init__(self, seconds: int) -> None:
        self.moment = datetime.fromtimestamp(seconds, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


class Clock:
    def __init__(self, values: list[int]) -> None:
        self.remaining = iter(values)

    def __call__(self) -> float:
        return float(next(self.remaining, 0))


def _environment(scenario: Scenario) -> dict[str, str]:
    environment = {
        "LITELLM_RUST": "false",
        "AWS_CONFIG_FILE": os.devnull,
        "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
        "AWS_ACCESS_KEY_ID": "AKIDEXAMPLE",
        "AWS_SECRET_ACCESS_KEY": TEST_KEY,
    }
    credentials = scenario.credentials
    if credentials is not None:
        environment["AWS_ACCESS_KEY_ID"] = credentials.access_key_id
        environment["AWS_SECRET_ACCESS_KEY"] = credentials.secret_access_key
        if credentials.session_token is not None:
            environment["AWS_SESSION_TOKEN"] = credentials.session_token
    environment.update(scenario.environment)
    return environment


def _transform(scenario: Scenario) -> SignedRequest:
    os.environ.clear()
    os.environ.update(_environment(scenario))
    litellm_uuid.uuid4 = FixedUuid(scenario.uuid)
    botocore.auth.get_current_datetime = FixedNow(scenario.now)  # type: ignore[misc]  # https://github.com/boto/botocore/blob/879f8440a4e9ace5d3cf145ce8b3d5e5ffb892ef/botocore/auth.py
    batch_request: CreateBatchRequest = {}
    if scenario.input_file_id is not None:
        batch_request["input_file_id"] = scenario.input_file_id
    if scenario.completion_window is not None:
        batch_request["completion_window"] = scenario.completion_window  # type: ignore[typeddict-item]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L522
    config = BedrockBatchesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L104
    result = config.transform_create_batch_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L188
        model=scenario.model,
        create_batch_data=batch_request,
        optional_params=scenario.optional_params,
        litellm_params=scenario.litellm_params,
    )
    return TypeAdapter(SignedRequest).validate_python(result)


def _request_call(scenario: Scenario) -> JsonValue:
    try:
        signed = _transform(scenario)
    except ValueError:
        return {"failed": True}
    return {"failed": False, "request": JSON.validate_json(signed.data)}


def _signed_call(scenario: Scenario) -> JsonValue:
    if scenario.credentials is None:
        return {"failed": True}
    try:
        signed = _transform(scenario)
    except ValueError:
        return {"failed": True}
    return {
        "failed": False,
        "method": signed.method,
        "url": signed.url,
        "headers": JSON.validate_python(signed.headers),
        "data": signed.data,
    }


def _reply_call(scenario: Scenario) -> JsonValue:
    litellm_params: dict[str, JsonValue] = {}
    if scenario.original_batch_request is not None:
        litellm_params["original_batch_request"] = scenario.original_batch_request
    response = httpx.Response(status_code=200, content=scenario.response_body.encode())
    config = BedrockBatchesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L104
    time.time = Clock(scenario.clock)
    try:
        reply = config.transform_create_batch_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L301
            model=None,
            raw_response=response,
            logging_obj=MagicMock(),
            litellm_params=litellm_params,
        )
    except ValueError:
        return {"failed": True}
    finally:
        time.time = REAL_TIME
    return {"failed": False, "reply": JSON.validate_json(reply.model_dump_json())}


def _titan_value(value: JsonValue) -> JsonValue:
    usage = None
    match value:
        case dict():
            usage = titan_embedding_usage_from_batch_output(value)
        case None | bool() | int() | float() | str() | list():
            usage = None
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }


SCENARIO_OPERATIONS: Final[dict[str, Callable[[Scenario], JsonValue]]] = {
    "batch_role_required": _request_call,
    "output_bucket_fallback": _request_call,
    "tags_validation": _request_call,
    "input_24h_window": _request_call,
    "generated_job_name": _request_call,
    "owner_and_kms": _request_call,
    "create_signed_request": _signed_call,
    "create_status_mapping": _reply_call,
    "create_reply_defaults": _reply_call,
}


def run(argument: JsonValue) -> JsonValue:
    input_value = TypeAdapter(Input).validate_python(argument)
    match input_value.operation:
        case "metadata_sanitization":
            return {
                "values": [
                    JSON.validate_python(BedrockBatchesConfig._get_openai_compatible_batch_metadata(value))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L376
                    for value in input_value.values
                ]
            }
        case "titan_batch_token_usage":
            return {"values": [_titan_value(value) for value in input_value.values]}
        case _:
            scenario_operation = SCENARIO_OPERATIONS[input_value.operation]
            return {"calls": [scenario_operation(scenario) for scenario in input_value.scenarios]}
