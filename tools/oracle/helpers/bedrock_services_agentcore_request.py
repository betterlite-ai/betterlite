import os

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import TYPE_CHECKING, Literal, TypeAlias, cast
from unittest import mock
from uuid import UUID

import botocore.auth
from botocore.credentials import Credentials
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.chat.agentcore import transformation
from litellm.llms.bedrock.chat.agentcore.transformation import AmazonAgentCoreConfig
from pydantic import BaseModel, ConfigDict, TypeAdapter

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class UrlRun(Strict):
    model: str
    api_base: str | None = None
    optional_params: dict[str, Json] = {}


class TransformRun(Strict):
    model: str
    messages: list[Json]
    optional_params: dict[str, Json] = {}
    litellm_params: dict[str, Json] = {}
    headers: list[tuple[str, str]] = []
    uuid: str


class ErrorRun(Strict):
    status: int
    message: str
    headers: list[tuple[str, str]]


class CredentialRun(Strict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None


class SignRun(Strict):
    model: str
    api_base: str
    headers: list[tuple[str, str]] = []
    optional_params: dict[str, Json] = {}
    request_data: dict[str, Json]
    api_key: str | None = None
    configured_bearer: str | None = None
    credentials: CredentialRun
    now: int


class RuntimeArnRequired(Strict):
    scenario: Literal["runtime_arn_required"]
    models: list[str]


class ArnRegion(Strict):
    scenario: Literal["arn_region"]
    runs: list[UrlRun]


class RuntimeSessionId(Strict):
    scenario: Literal["runtime_session_id"]
    runs: list[TransformRun]


class RuntimeUserId(Strict):
    scenario: Literal["runtime_user_id"]
    runs: list[TransformRun]


class MultimodalForwarding(Strict):
    scenario: Literal["multimodal_forwarding"]
    runs: list[TransformRun]


class QualifierQuery(Strict):
    scenario: Literal["qualifier_query"]
    runs: list[UrlRun]


class StreamBodyPolicy(Strict):
    scenario: Literal["stream_body_policy"]
    optional_params: dict[str, Json]
    headers: list[tuple[str, str]]
    error: ErrorRun


class AgentcoreBearerSigning(Strict):
    scenario: Literal["agentcore_bearer_signing"]
    runs: list[SignRun]


class AgentcoreIamSigning(Strict):
    scenario: Literal["agentcore_iam_signing"]
    runs: list[SignRun]


def _scenario(argument: Json) -> str:
    if not isinstance(argument, dict):
        raise ValueError("the helper input must be an object")
    scenario = argument.get("scenario")
    if not isinstance(scenario, str):
        raise ValueError("the helper input needs a scenario name")
    return scenario


def _error_kind(message: str) -> str:
    if message.startswith("Invalid model format"):
        return "invalid_model_format"
    if message.startswith("Invalid ARN format"):
        return "invalid_arn"
    raise ValueError(message)


def _url_result(model: str, api_base: str | None, optional_params: dict[str, Json]) -> Json:
    config = AmazonAgentCoreConfig()
    try:
        url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportUnknownArgumentType]
            api_base=api_base,
            api_key=None,
            model=model,
            optional_params=dict(optional_params),
            litellm_params={},
        )
    except ValueError as error:
        return {
            "api_base": api_base,
            "ok": False,
            "error": _error_kind(str(error)),
        }
    return {"api_base": api_base, "ok": True, "url": url}


def _transform_result(case: TransformRun) -> Json:
    config = AmazonAgentCoreConfig()
    headers = dict(case.headers)
    messages = cast("list[AllMessageValues]", case.messages)
    fixed = UUID(case.uuid)
    with mock.patch.object(
        transformation, "uuid", SimpleNamespace(uuid4=lambda: fixed)
    ):
        try:
            payload = config.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportUnknownArgumentType]
                model=case.model,
                messages=messages,
                optional_params=dict(case.optional_params),
                litellm_params=dict(case.litellm_params),
                headers=headers,
            )
        except IndexError:
            return {"ok": False, "model": case.model, "error": "empty_messages"}
    return cast(
        Json,
        {
            "ok": True,
            "model": case.model,
            "payload": cast(Json, payload),
            "headers": headers,
        },
    )


@contextmanager
def _harness(
    configured_bearer: str | None, resolve: Callable[[object, object], Credentials]
) -> Generator[None, None, None]:
    def no_secret(_name: str, default: object = None) -> object:
        return default

    def bearer_secret(name: str) -> str | None:
        return configured_bearer if name == "AWS_BEARER_TOKEN_BEDROCK" else None

    def resolver(_self: object, auth: object, region: object) -> Credentials:
        return resolve(auth, region)

    with (
        mock.patch.dict(os.environ, {}, clear=True),
        mock.patch.object(base_aws_llm, "get_secret", no_secret),
        mock.patch.object(base_aws_llm, "get_secret_str", bearer_secret),
        mock.patch(
            "boto3.Session", return_value=SimpleNamespace(region_name=None)
        ),
        mock.patch.object(BaseAWSLLM, "resolve_credentials", resolver),
    ):
        yield


@contextmanager
def _clock(frozen: datetime) -> Generator[None, None, None]:
    with mock.patch.object(
        botocore.auth, "get_current_datetime", return_value=frozen
    ):
        yield


def _sign_result(case: SignRun) -> Json:
    config = AmazonAgentCoreConfig()
    credentials = Credentials(
        access_key=case.credentials.access_key_id,
        secret_key=case.credentials.secret_access_key,
        token=case.credentials.session_token,
    )
    frozen = datetime.fromtimestamp(case.now, tz=UTC).replace(tzinfo=None)

    def resolve(_auth: object, _region: object) -> Credentials:
        return credentials

    with _harness(case.configured_bearer, resolve), _clock(frozen):
        try:
            headers, body = config.sign_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportUnknownArgumentType]
                headers=dict(case.headers),
                optional_params=dict(case.optional_params),
                request_data=dict(case.request_data),
                api_base=case.api_base,
                api_key=case.api_key,
                model=case.model,
            )
        except ValueError as error:
            return {"ok": False, "error": _error_kind(str(error))}
    return {
        "ok": True,
        "method": "POST",
        "url": case.api_base,
        "headers": headers,
        "body": "" if body is None else body.decode(),
    }


def _stream_body_result(case: StreamBodyPolicy) -> Json:
    config = AmazonAgentCoreConfig()
    supported = config.get_supported_openai_params(model="agentcore")
    mapped = config.map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportUnknownArgumentType]
        {}, dict(case.optional_params), "agentcore", drop_params=False
    )
    validated = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportUnknownArgumentType]
        dict(case.headers), "agentcore", [], dict(case.optional_params), {}
    )
    error = config.get_error_class(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        case.error.message, case.error.status, dict(case.error.headers)
    )
    return cast(
        Json,
        {
            "supported": supported,
            "mapped": cast(Json, mapped),
            "has_custom_stream_wrapper": config.has_custom_stream_wrapper,
            "supports_stream_param_in_request_body": config.supports_stream_param_in_request_body,
            "should_fake_stream": config.should_fake_stream(model="agentcore", stream=True),
            "headers": cast(Json, validated),
            "error": cast(
                Json,
                {
                    "status_code": error.status_code,
                    "status_code_is_synthesized": error.status_code_is_synthesized,
                    "headers": cast(Json, error.headers),
                    "response_status": error.response.status_code,
                },
            ),
        },
    )



TransformScenario: TypeAlias = RuntimeSessionId | RuntimeUserId | MultimodalForwarding
SignScenario: TypeAlias = AgentcoreBearerSigning | AgentcoreIamSigning


def run(argument: Json) -> Json:
    scenario = _scenario(argument)
    match scenario:
        case "runtime_arn_required":
            case = TypeAdapter(RuntimeArnRequired).validate_python(argument)
            return {
                "results": [
                    _url_result(model, None, {}) for model in case.models
                ],
            }
        case "arn_region":
            arn_case = TypeAdapter(ArnRegion).validate_python(argument)
            return {
                "results": [
                    _url_result(run.model, run.api_base, run.optional_params)
                    for run in arn_case.runs
                ],
            }
        case "runtime_session_id" | "runtime_user_id" | "multimodal_forwarding":
            transform_case = TypeAdapter(TransformScenario).validate_python(argument)
            return {
                "results": [
                    _transform_result(run) for run in transform_case.runs
                ],
            }
        case "qualifier_query":
            qualifier_case = TypeAdapter(QualifierQuery).validate_python(argument)
            return {
                "results": [
                    _url_result(run.model, run.api_base, run.optional_params)
                    for run in qualifier_case.runs
                ],
            }
        case "stream_body_policy":
            return _stream_body_result(
                TypeAdapter(StreamBodyPolicy).validate_python(argument)
            )
        case "agentcore_bearer_signing" | "agentcore_iam_signing":
            sign_case = TypeAdapter(SignScenario).validate_python(argument)
            return {
                "results": [_sign_result(run) for run in sign_case.runs],
            }
        case _:
            raise ValueError(f"unknown scenario: {scenario}")
