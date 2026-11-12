from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L433
import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.batches import transformation
from litellm.llms.bedrock.batches.handler import (
    _extract_job_id_from_arn,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L53
    _extract_region_from_bedrock_arn,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L42
    _predict_output_file_uri,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L60
    _record_counts_from_response,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L82
    _sigv4_config,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L36
    _to_epoch,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L95
)
from litellm.types.llms.bedrock import AwsAuthParams
from litellm.types.utils import LiteLLMBatch

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

PROJECTED_KEYS: Final = (
    "id", "object", "endpoint", "completion_window",
    "status", "created_at", "in_progress_at", "completed_at",
    "failed_at", "cancelled_at", "expires_at", "finalizing_at",
    "expired_at", "cancelling_at", "error_file_id", "input_file_id",
    "output_file_id", "request_counts", "errors", "metadata",
    "model", "usage",
)


class CredentialKeys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class Retrieve(TypedDict):
    batch_id: str
    litellm_params: dict[str, Json]
    optional_params: dict[str, Json]
    now: int


class Prediction(TypedDict):
    output_prefix: str
    input_uri: str
    job_id: NotRequired[str | None]


class DecodeInput(TypedDict):
    operation: Literal["decode"]
    now: int
    status_code: int
    bodies: list[str]


class RecordCountsInput(TypedDict):
    operation: Literal["record_counts"]
    responses: list[dict[str, Json]]


class OutputUriPredictionInput(TypedDict):
    operation: Literal["output_uri_prediction"]
    arns: list[str]
    predictions: list[Prediction]


class RetrieveSignedRequestInput(TypedDict):
    operation: Literal["retrieve_signed_request"]
    credentials: CredentialKeys
    requests: list[Retrieve]


class RegionAndTimeInput(TypedDict):
    operation: Literal["region_and_time"]
    arns: list[str]
    times: list["TimeInput"]


class TimeInput(TypedDict, total=False):
    integer: int
    float: float
    boolean: bool
    datetime: str
    text: str
    null: None
    nonfinite: str


Input: TypeAlias = (
    DecodeInput
    | RecordCountsInput
    | OutputUriPredictionInput
    | RetrieveSignedRequestInput
    | RegionAndTimeInput
)

PythonTime: TypeAlias = int | float | bool | datetime | None | str


class FrozenClock:
    def __init__(self, epoch: int) -> None:
        self.moment = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


class FrozenEpoch:
    def __init__(self, epoch: int) -> None:
        self.epoch = float(epoch)

    def __call__(self) -> float:
        return self.epoch


class FixedCredentials:
    access_key: str
    secret_key: str
    token: str | None

    def __init__(self, keys: CredentialKeys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token = keys.get("session_token")


class Resolver:
    def __init__(self, credentials: FixedCredentials) -> None:
        self.credentials = credentials

    def __call__(self, _auth_params: AwsAuthParams, _region: str | None) -> FixedCredentials:
        return self.credentials


class SignatureConfig(Protocol):
    signature_version: str


def project_batch(batch: LiteLLMBatch) -> Json:
    dumped = cast(dict[str, Json], batch.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4320
    return {key: dumped.get(key) for key in PROJECTED_KEYS}


def decode_bodies(case: DecodeInput) -> Json:
    config = transformation.BedrockBatchesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L104
    transformation.time = SimpleNamespace(time=FrozenEpoch(case["now"]))  # type: ignore[assignment, attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L642
    params: dict[str, Json] = {}
    responses = [
        httpx.Response(status_code=case["status_code"], content=body.encode())
        for body in case["bodies"]
    ]
    values: list[Json] = []
    for response in responses:
        try:
            batch = config.transform_retrieve_batch_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L569
                model=None,
                raw_response=response,
                logging_obj=cast(Logging, None),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L573
                litellm_params=params,
            )
        except (AttributeError, ValueError):
            values.append({"ok": False})
        else:
            values.append({"ok": True, "batch": project_batch(batch)})
    return {"values": values}


def record_counts(case: RecordCountsInput) -> Json:
    values: list[Json] = []
    for response in case["responses"]:
        counts = _record_counts_from_response(response)
        if counts is None:
            values.append(None)
        else:
            values.append(
                {"completed": counts.completed, "failed": counts.failed, "total": counts.total}
            )
    return {"counts": values}


def output_uris(case: OutputUriPredictionInput) -> Json:
    job_ids: list[Json] = [_extract_job_id_from_arn(arn) for arn in case["arns"]]
    uris: list[Json] = [
        _predict_output_file_uri(entry["output_prefix"], entry["input_uri"], entry.get("job_id"))
        for entry in case["predictions"]
    ]
    return {"job_ids": job_ids, "uris": uris}


def retrieve_signed(case: RetrieveSignedRequestInput) -> Json:
    config = transformation.BedrockBatchesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L104
    BaseAWSLLM.resolve_credentials = Resolver(FixedCredentials(case["credentials"]))  # type: ignore[method-assign, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    values: list[Json] = []
    for request in case["requests"]:
        botocore.auth.get_current_datetime = FrozenClock(request["now"])  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L433
        try:
            result = config.transform_retrieve_batch_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L396
                batch_id=request["batch_id"],
                optional_params=request["optional_params"],
                litellm_params=request["litellm_params"],
            )
        except ValueError:
            values.append({"ok": False})
            continue
        signed = cast(dict[str, Json], result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/transformation.py#L396
        values.append(
            {
                "ok": True,
                "method": signed["method"],
                "url": signed["url"],
                "headers": signed["headers"],
                "data": signed["data"],
            }
        )
    return {"values": values}


def nonfinite_value(kind: str) -> float:
    if kind == "inf":
        return float("inf")
    if kind == "-inf":
        return float("-inf")
    if kind == "nan":
        return float("nan")
    raise ValueError("the nonfinite value is unknown")


def python_value(value: TimeInput) -> PythonTime:
    flag = value.get("boolean")
    number = value.get("integer")
    real = value.get("float")
    text = value.get("text")
    stamp = value.get("datetime")
    kind = value.get("nonfinite")
    if flag is not None:
        result: PythonTime = flag
    elif number is not None:
        result = number
    elif real is not None:
        result = real
    elif text is not None:
        result = text
    elif stamp is not None:
        result = datetime.fromisoformat(stamp)
    elif kind is not None:
        result = nonfinite_value(kind)
    elif "null" in value:
        result = None
    else:
        raise ValueError("the time value has no tag")
    return result


def epoch_value(value: TimeInput) -> Json:
    moment = python_value(value)
    try:
        epoch = _to_epoch(moment)
    except (ValueError, OverflowError):
        return {"failed": True}
    return epoch


def region_and_time(case: RegionAndTimeInput) -> Json:
    regions: list[Json] = [_extract_region_from_bedrock_arn(arn) for arn in case["arns"]]
    epochs: list[Json] = [epoch_value(value) for value in case["times"]]
    config = cast(SignatureConfig, _sigv4_config())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L36
    return {
        "regions": regions,
        "epochs": epochs,
        "signature_version": config.signature_version,
    }


def run(input_value: Input) -> Json:
    match input_value["operation"]:
        case "decode":
            return decode_bodies(input_value)
        case "record_counts":
            return record_counts(input_value)
        case "output_uri_prediction":
            return output_uris(input_value)
        case "retrieve_signed_request":
            return retrieve_signed(input_value)
        case "region_and_time":
            return region_and_time(input_value)
