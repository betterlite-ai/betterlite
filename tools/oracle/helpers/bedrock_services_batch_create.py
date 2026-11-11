import os

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import TYPE_CHECKING, Final, TypeAlias, cast
from unittest.mock import MagicMock, patch
from uuid import UUID

import botocore.auth
import httpx
import litellm
from botocore.credentials import Credentials
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.batches.transformation import (
    BedrockBatchesConfig,
    titan_embedding_usage_from_batch_output,
)
from pydantic import BaseModel, ConfigDict

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
    from litellm.types.llms.openai import CreateBatchRequest

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
DEFAULT_UUID: Final = "00000000-0000-4000-8000-000000000000"


class CredentialsInput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    access_key_id: str
    secret_access_key: str
    session_token: str | None = None


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str | None = None
    input_file_id: str | None = None
    completion_window: str | None = None
    litellm_params: dict[str, Json] = {}
    optional_params: dict[str, Json] = {}
    environment: dict[str, str] = {}
    original_batch_request: Json = None
    response_body: str | None = None
    clock: list[int] = []
    job_prefix: str | None = None
    uuid: str | None = None
    credentials: CredentialsInput | None = None
    now: int | None = None


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: str
    scenarios: list[Scenario] = []
    values: list[Json] = []


@contextmanager
def _environment(scenario: Scenario) -> Generator[None, None, None]:
    values = {"LITELLM_RUST": "false", **scenario.environment}
    with patch.dict(os.environ, values, clear=True):
        yield


@contextmanager
def _fixed_uuid(value: str | None) -> Generator[None, None, None]:
    fixed = UUID(value or DEFAULT_UUID)
    with patch.object(
        litellm_uuid,
        "uuid4",
        lambda: fixed,
    ):  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
        yield


def _create_batch_data(scenario: Scenario) -> dict[str, Json]:
    data: dict[str, Json] = {}
    if scenario.input_file_id is not None:
        data["input_file_id"] = scenario.input_file_id
    if scenario.completion_window is not None:
        data["completion_window"] = scenario.completion_window
    return data


def _params(scenario: Scenario) -> tuple[dict[str, object], dict[str, object]]:
    deployment = cast("dict[str, object]", dict(scenario.litellm_params))
    request = cast("dict[str, object]", dict(scenario.optional_params))
    return deployment, request


def _build_call(scenario: Scenario) -> Json:
    config = BedrockBatchesConfig()
    signer = MagicMock()
    deployment, request = _params(scenario)
    with (
        _environment(scenario),
        _fixed_uuid(scenario.uuid),
        patch.object(config.common_utils, "sign_aws_request", signer),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
    ):
        signer.return_value = ({}, b"")
        try:
            config.transform_create_batch_request(  # pyright: ignore[reportUnknownMemberType]
                model=scenario.model or "",
                create_batch_data=cast(
                    "CreateBatchRequest", _create_batch_data(scenario)
                ),
                optional_params=request,
                litellm_params=deployment,
            )
        except ValueError:
            return cast(Json, {"failed": True})
    call = cast("MagicMock", signer.call_args)
    return cast(Json, {"failed": False, "request": cast(Json, call.kwargs["data"])})


def _build_operation(input_value: Input) -> Json:
    calls = [_build_call(scenario) for scenario in input_value.scenarios]
    return cast(Json, {"calls": calls})


def _signed_call(scenario: Scenario) -> Json:
    if scenario.credentials is None:
        return cast(Json, {"failed": True})
    credentials = scenario.credentials
    boto_credentials = Credentials(
        access_key=credentials.access_key_id,
        secret_key=credentials.secret_access_key,
        token=credentials.session_token,
    )
    config = BedrockBatchesConfig()
    clock = datetime.fromtimestamp(scenario.now or 0, tz=UTC).replace(tzinfo=None)
    deployment, request = _params(scenario)
    with (
        _environment(scenario),
        _fixed_uuid(scenario.uuid),
        patch.object(botocore.auth, "get_current_datetime", lambda: clock),
        patch.object(BaseAWSLLM, "resolve_credentials", return_value=boto_credentials),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
    ):
        try:
            result = config.transform_create_batch_request(  # pyright: ignore[reportUnknownMemberType]
                model=scenario.model or "",
                create_batch_data=cast(
                    "CreateBatchRequest", _create_batch_data(scenario)
                ),
                optional_params=request,
                litellm_params=deployment,
            )
        except Exception:  # noqa: BLE001 - a refused fixture call is one failed outcome
            return cast(Json, {"failed": True})
    return cast(
        Json,
        {
            "failed": False,
            "method": cast(Json, result["method"]),
            "url": cast(Json, result["url"]),
            "headers": cast(Json, result["headers"]),
            "data": cast(Json, result["data"]),
        },
    )


def _signed_operation(input_value: Input) -> Json:
    calls = [_signed_call(scenario) for scenario in input_value.scenarios]
    return cast(Json, {"calls": calls})


def _clock(values: list[int]) -> Callable[[], float]:
    remaining = iter(values)

    def current() -> float:
        return float(next(remaining, 0))

    return current


def _reply_call(scenario: Scenario) -> Json:
    litellm_params: dict[str, object] = {}
    if scenario.original_batch_request is not None:
        litellm_params["original_batch_request"] = scenario.original_batch_request
    response = httpx.Response(
        status_code=200,
        content=(scenario.response_body or "").encode(),
    )
    config = BedrockBatchesConfig()
    with patch(
        "litellm.llms.bedrock.batches.transformation.time.time",
        _clock(scenario.clock),
    ):
        try:
            reply = config.transform_create_batch_response(  # pyright: ignore[reportUnknownMemberType]
                model=None,
                raw_response=response,
                logging_obj=cast("LiteLLMLoggingObj", MagicMock()),
                litellm_params=litellm_params,
            )
        except Exception:  # noqa: BLE001 - a refused fixture body is one failed outcome
            return cast(Json, {"failed": True})
    return cast(Json, {"failed": False, "reply": cast(Json, reply.model_dump())})


def _reply_operation(input_value: Input) -> Json:
    calls = [_reply_call(scenario) for scenario in input_value.scenarios]
    return cast(Json, {"calls": calls})


def _metadata(input_value: Input) -> Json:
    values = [
        cast(
            Json,
            BedrockBatchesConfig._get_openai_compatible_batch_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                value
            ),
        )
        for value in input_value.values
    ]
    return cast(Json, {"values": values})


def _titan(input_value: Input) -> Json:
    values: list[Json] = []
    for value in input_value.values:
        if not isinstance(value, dict):
            values.append(None)
            continue
        usage = titan_embedding_usage_from_batch_output(
            cast("dict[str, object]", value)
        )
        if usage is None:
            values.append(None)
            continue
        values.append(
            cast(
                Json,
                {
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                },
            )
        )
    return cast(Json, {"values": values})


HANDLERS: Final[dict[str, Callable[[Input], Json]]] = {
    "batch_role_required": _build_operation,
    "output_bucket_fallback": _build_operation,
    "tags_validation": _build_operation,
    "input_24h_window": _build_operation,
    "generated_job_name": _build_operation,
    "owner_and_kms": _build_operation,
    "create_signed_request": _signed_operation,
    "create_status_mapping": _reply_operation,
    "create_reply_defaults": _reply_operation,
    "metadata_sanitization": _metadata,
    "titan_batch_token_usage": _titan,
}


def run(argument: Json) -> Json:
    input_value = Input.model_validate(argument)
    handler = HANDLERS.get(input_value.operation)
    if handler is None:
        raise ValueError(f"unknown operation {input_value.operation}")
    return handler(input_value)
