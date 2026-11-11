import os
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Annotated, Final, Literal, Protocol, TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import httpx
import litellm
from botocore.credentials import Credentials
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.batches.handler import (
    _extract_job_id_from_arn,  # pyright: ignore[reportPrivateUsage]
    _extract_region_from_bedrock_arn,  # pyright: ignore[reportPrivateUsage]
    _predict_output_file_uri,  # pyright: ignore[reportPrivateUsage]
    _record_counts_from_response,  # pyright: ignore[reportPrivateUsage]
    _sigv4_config,  # pyright: ignore[reportPrivateUsage]
    _to_epoch,  # pyright: ignore[reportPrivateUsage]
)
from litellm.llms.bedrock.batches.transformation import BedrockBatchesConfig
from litellm.types.utils import LiteLLMBatch
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Json: TypeAlias = JsonValue

PROJECTED_KEYS: Final = (
    "id",
    "object",
    "endpoint",
    "completion_window",
    "status",
    "created_at",
    "in_progress_at",
    "completed_at",
    "failed_at",
    "cancelled_at",
    "expires_at",
    "finalizing_at",
    "expired_at",
    "cancelling_at",
    "error_file_id",
    "input_file_id",
    "output_file_id",
    "request_counts",
    "errors",
    "metadata",
    "model",
    "usage",
)


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class CredentialKeys(Strict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None

    def to_boto(self) -> Credentials:
        return Credentials(
            access_key=self.access_key_id,
            secret_key=self.secret_access_key,
            token=self.session_token,
        )


class Retrieve(Strict):
    batch_id: str
    litellm_params: dict[str, Json]
    optional_params: dict[str, Json]
    now: int


class Prediction(Strict):
    output_prefix: str
    input_uri: str
    job_id: str | None = None


class DecodeInput(Strict):
    operation: Literal["decode"]
    now: int
    status_code: int
    bodies: list[str]


class RecordCountsInput(Strict):
    operation: Literal["record_counts"]
    responses: list[dict[str, Json]]


class OutputUriPredictionInput(Strict):
    operation: Literal["output_uri_prediction"]
    arns: list[str]
    predictions: list[Prediction]


class RetrieveSignedRequestInput(Strict):
    operation: Literal["retrieve_signed_request"]
    credentials: CredentialKeys
    requests: list[Retrieve]


class IntegerTime(Strict):
    integer: int


class FloatTime(Strict):
    float: float


class BooleanTime(Strict):
    boolean: bool


class DatetimeTime(Strict):
    datetime: str


class TextTime(Strict):
    text: str


class NullTime(Strict):
    null: None


class NonfiniteTime(Strict):
    nonfinite: str


TimeValue: TypeAlias = (
    IntegerTime | FloatTime | BooleanTime | DatetimeTime | TextTime | NullTime | NonfiniteTime
)


class RegionAndTimeInput(Strict):
    operation: Literal["region_and_time"]
    arns: list[str]
    times: list[TimeValue]


Operation: TypeAlias = Annotated[
    DecodeInput
    | RecordCountsInput
    | OutputUriPredictionInput
    | RetrieveSignedRequestInput
    | RegionAndTimeInput,
    Field(discriminator="operation"),
]

OPERATION: Final[TypeAdapter[Operation]] = TypeAdapter(Operation)


def secret_lookup(_name: str, default: str | None = None) -> str | None:
    return default


def bearer_lookup(_name: str, _default: str | None = None) -> str | None:
    return None


class BatchesConfig(Protocol):
    def transform_retrieve_batch_request(
        self,
        batch_id: str,
        optional_params: Mapping[str, object],
        litellm_params: Mapping[str, object],
    ) -> dict[str, object]: ...

    def transform_retrieve_batch_response(
        self,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Mapping[str, object],
    ) -> LiteLLMBatch: ...


class SignatureConfig(Protocol):
    signature_version: str


class Resolver:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials

    def __call__(self, _auth_params: object, _region: object) -> Credentials:
        return self.credentials


@contextmanager
def clock(epoch: int) -> Generator[None, None, None]:
    frozen = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)
    with patch.object(botocore.auth, "get_current_datetime", lambda: frozen):
        yield


@contextmanager
def frozen_time(epoch: int) -> Generator[None, None, None]:
    with patch(
        "litellm.llms.bedrock.batches.transformation.time.time", return_value=float(epoch)
    ):
        yield


@contextmanager
def harness(resolver: Resolver) -> Generator[None, None, None]:
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(base_aws_llm, "get_secret", secret_lookup),
        patch.object(base_aws_llm, "get_secret_str", bearer_lookup),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
        patch.object(BaseAWSLLM, "resolve_credentials", resolver),
    ):
        yield


def project_batch(batch: LiteLLMBatch) -> Json:
    dumped = cast(dict[str, Json], batch.model_dump())
    return {key: dumped.get(key) for key in PROJECTED_KEYS}


def decode_bodies(case: DecodeInput) -> Json:
    config = cast(BatchesConfig, BedrockBatchesConfig())
    values: list[Json] = []
    for body in case.bodies:
        response = httpx.Response(status_code=case.status_code, content=body.encode())
        try:
            with frozen_time(case.now):
                batch = config.transform_retrieve_batch_response(
                    model=None,
                    raw_response=response,
                    logging_obj=cast(Logging, None),
                    litellm_params={},
                )
        except Exception:  # noqa: BLE001 - a malformed body or record shape reports instead of aborting
            values.append({"ok": False})
            continue
        values.append({"ok": True, "batch": project_batch(batch)})
    return {"values": values}


def record_counts(case: RecordCountsInput) -> Json:
    values: list[Json] = []
    for response in case.responses:
        counts = _record_counts_from_response(response)
        values.append(None if counts is None else cast(Json, counts.model_dump()))
    return {"counts": values}


def output_uris(case: OutputUriPredictionInput) -> Json:
    job_ids: list[Json] = [_extract_job_id_from_arn(arn) for arn in case.arns]
    uris: list[Json] = [
        _predict_output_file_uri(entry.output_prefix, entry.input_uri, entry.job_id)
        for entry in case.predictions
    ]
    return cast(Json, {"job_ids": job_ids, "uris": uris})


def retrieve_signed(case: RetrieveSignedRequestInput) -> Json:
    config = cast(BatchesConfig, BedrockBatchesConfig())
    values: list[Json] = []
    for request in case.requests:
        try:
            with harness(Resolver(case.credentials.to_boto())), clock(request.now):
                result = config.transform_retrieve_batch_request(
                    batch_id=request.batch_id,
                    optional_params=dict(request.optional_params),
                    litellm_params=dict(request.litellm_params),
                )
        except Exception:  # noqa: BLE001 - an invalid ARN or signing input reports as a failure
            values.append({"ok": False})
            continue
        signed = cast(Mapping[str, object], result)
        values.append(
            cast(
                Json,
                {
                    "ok": True,
                    "method": signed["method"],
                    "url": signed["url"],
                    "headers": signed["headers"],
                    "data": signed["data"],
                },
            )
        )
    return {"values": values}


def nonfinite_value(kind: str) -> float:
    match kind:
        case "inf":
            return float("inf")
        case "-inf":
            return float("-inf")
        case "nan":
            return float("nan")
        case _:
            raise ValueError(f"unknown nonfinite value {kind}")


def python_value(value: TimeValue) -> object:
    match value:
        case IntegerTime(integer=number):
            return number
        case FloatTime(float=number):
            return number
        case BooleanTime(boolean=flag):
            return flag
        case _:
            return python_text_value(value)


def python_text_value(value: DatetimeTime | TextTime | NullTime | NonfiniteTime) -> object:
    match value:
        case DatetimeTime(datetime=text):
            return datetime.fromisoformat(text)
        case TextTime(text=text):
            return text
        case NullTime():
            return None
        case NonfiniteTime(nonfinite=kind):
            return nonfinite_value(kind)


def epoch_value(value: TimeValue) -> Json:
    try:
        epoch = _to_epoch(python_value(value))
    except Exception:  # noqa: BLE001 - a nonfinite float has no epoch and reports as a failure
        return {"failed": True}
    return epoch


def region_and_time(case: RegionAndTimeInput) -> Json:
    regions: list[Json] = [_extract_region_from_bedrock_arn(arn) for arn in case.arns]
    epochs: list[Json] = [epoch_value(value) for value in case.times]
    config = cast(SignatureConfig, _sigv4_config())
    return cast(
        Json,
        {
            "regions": regions,
            "epochs": epochs,
            "signature_version": config.signature_version,
        },
    )


def run(input_value: dict[str, Json]) -> Json:
    case = OPERATION.validate_python(input_value)
    match case:
        case DecodeInput():
            return decode_bodies(case)
        case RecordCountsInput():
            return record_counts(case)
        case OutputUriPredictionInput():
            return output_uris(case)
        case RetrieveSignedRequestInput():
            return retrieve_signed(case)
        case RegionAndTimeInput():
            return region_and_time(case)
