import os
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict
from uuid import UUID

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
from litellm.llms.bedrock.chat.agentcore.transformation import AmazonAgentCoreConfig

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Params: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, str]

SESSION_HEADER = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id"
GENERATED_SESSION_PREFIX = "litellm-session-"


class UrlRun(TypedDict):
    model: str
    api_base: NotRequired[str | None]
    optional_params: NotRequired[Params]


class TransformRun(TypedDict):
    model: str
    messages: list[Json]
    optional_params: NotRequired[Params]
    litellm_params: NotRequired[Params]
    headers: NotRequired[list[tuple[str, str]]]
    uuid: str


class ErrorRun(TypedDict):
    status: int
    message: str
    headers: list[tuple[str, str]]


class CredentialRun(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class SignRun(TypedDict):
    model: str
    api_base: str
    headers: NotRequired[list[tuple[str, str]]]
    optional_params: NotRequired[Params]
    request_data: Params
    api_key: NotRequired[str | None]
    configured_bearer: NotRequired[str | None]
    credentials: CredentialRun
    now: int


class RuntimeArnRequired(TypedDict):
    scenario: Literal["runtime_arn_required"]
    models: list[str]


class UrlScenario(TypedDict):
    scenario: Literal["arn_region", "qualifier_query"]
    runs: list[UrlRun]


class TransformScenario(TypedDict):
    scenario: Literal["runtime_session_id", "runtime_user_id", "multimodal_forwarding"]
    runs: list[TransformRun]


class StreamBodyPolicy(TypedDict):
    scenario: Literal["stream_body_policy"]
    optional_params: Params
    headers: list[tuple[str, str]]
    error: ErrorRun


class SignScenario(TypedDict):
    scenario: Literal["agentcore_bearer_signing", "agentcore_iam_signing"]
    runs: list[SignRun]


Input: TypeAlias = (
    RuntimeArnRequired | UrlScenario | TransformScenario | StreamBodyPolicy | SignScenario
)


class ResponseLike(Protocol):
    status_code: int


class ErrorLike(Protocol):
    status_code: int
    status_code_is_synthesized: bool
    headers: Params
    response: ResponseLike


class AgentCoreApi(Protocol):
    def get_complete_url(
        self, base: str | None, key: str | None, model: str, params: Params, extra: Params, /
    ) -> str: ...

    def transform_request(
        self, model: str, messages: list[Json], params: Params, extra: Params, headers: Headers, /
    ) -> Params: ...

    def sign_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/agentcore/transformation.py#L116
        self,
        headers: Headers,
        params: Params,
        data: Params,
        base: str,
        key: str | None,
        model: str | None,
        /,
    ) -> tuple[Headers, bytes | None]: ...

    def get_supported_openai_params(self, model: str, /) -> list[str]: ...

    def map_openai_params(
        self, non_default: Params, params: Params, model: str, /, *, drop_params: bool
    ) -> Params: ...

    def validate_environment(
        self, headers: Headers, model: str, messages: list[Json], params: Params, extra: Params, /
    ) -> Headers: ...

    def get_error_class(self, message: str, status: int, headers: Headers, /) -> ErrorLike: ...

    def should_fake_stream(self, model: str | None, /, *, stream: bool | None) -> bool: ...


class FrozenClock:
    def __init__(self, frozen: datetime) -> None:
        self.frozen = frozen

    def __call__(self) -> datetime:
        return self.frozen


def _error_kind(message: str) -> str:
    if message.startswith("Invalid model format"):
        return "invalid_model_format"
    if message.startswith("Invalid ARN format"):
        return "invalid_arn"
    raise ValueError(message)


def _url_result(config: AgentCoreApi, run: UrlRun) -> Json:
    api_base = run.get("api_base")
    run_params = run.get("optional_params")
    optional_params: Params = {} if run_params is None else dict(run_params)
    litellm_params: Params = {}
    try:
        url = config.get_complete_url(api_base, None, run["model"], optional_params, litellm_params)
    except ValueError as error:
        return {"api_base": api_base, "ok": False, "error": _error_kind(str(error))}
    return {"api_base": api_base, "ok": True, "url": url}


def _transform_result(config: AgentCoreApi, run: TransformRun) -> Json:
    run_headers = run.get("headers")
    headers: Headers = {} if run_headers is None else dict(run_headers)
    run_params = run.get("optional_params")
    optional_params: Params = {} if run_params is None else dict(run_params)
    run_litellm_params = run.get("litellm_params")
    litellm_params: Params = {} if run_litellm_params is None else dict(run_litellm_params)
    try:
        payload = config.transform_request(
            run["model"], run["messages"], optional_params, litellm_params, headers
        )
    except IndexError:
        return {"ok": False, "model": run["model"], "error": "empty_messages"}
    if not optional_params.get("runtimeSessionId"):
        UUID(headers[SESSION_HEADER].removeprefix(GENERATED_SESSION_PREFIX))
        headers[SESSION_HEADER] = f"{GENERATED_SESSION_PREFIX}{run['uuid']}"
    return {"ok": True, "model": run["model"], "payload": payload, "headers": Params(headers)}


def _sign_result(config: AgentCoreApi, run: SignRun) -> Json:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull
    configured_bearer = run.get("configured_bearer")
    if configured_bearer is not None:
        os.environ["AWS_BEARER_TOKEN_BEDROCK"] = configured_bearer
    frozen = datetime.fromtimestamp(run["now"], tz=UTC).replace(tzinfo=None)
    botocore.auth.get_current_datetime = FrozenClock(frozen)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
    run_params = run.get("optional_params")
    optional_params: Params = {} if run_params is None else dict(run_params)
    optional_params["aws_access_key_id"] = run["credentials"]["access_key_id"]
    optional_params["aws_secret_access_key"] = run["credentials"]["secret_access_key"]
    session_token = run["credentials"].get("session_token")
    if session_token is not None:
        optional_params["aws_session_token"] = session_token
    run_headers = run.get("headers")
    request_headers: Headers = {} if run_headers is None else dict(run_headers)
    try:
        signed = config.sign_request(
            request_headers,
            optional_params,
            run["request_data"],
            run["api_base"],
            run.get("api_key"),
            run["model"],
        )
    except ValueError as error:
        return {"ok": False, "error": _error_kind(str(error))}
    headers, body = signed
    return {
        "ok": True,
        "method": "POST",
        "url": run["api_base"],
        "headers": Params(headers),
        "body": "" if body is None else body.decode(),
    }


def _stream_body_result(config: AgentCoreApi, case: StreamBodyPolicy, stream_flags: Params) -> Json:
    optional_params: Params = dict(case["optional_params"])
    request_headers: Headers = dict(case["headers"])
    error_headers: Headers = dict(case["error"]["headers"])
    supported = config.get_supported_openai_params("agentcore")
    no_messages: list[Json] = []
    no_litellm_params: Params = {}
    mapped = config.map_openai_params(
        no_litellm_params, optional_params, "agentcore", drop_params=False
    )
    validated = config.validate_environment(
        request_headers, "agentcore", no_messages, optional_params, no_litellm_params
    )
    error = config.get_error_class(case["error"]["message"], case["error"]["status"], error_headers)
    return {
        "supported": list[Json](supported),
        "mapped": mapped,
        **stream_flags,
        "should_fake_stream": config.should_fake_stream("agentcore", stream=True),
        "headers": Params(validated),
        "error": {
            "status_code": error.status_code,
            "status_code_is_synthesized": error.status_code_is_synthesized,
            "headers": error.headers,
            "response_status": error.response.status_code,
        },
    }


def run(input_value: Input) -> Json:
    agent = AmazonAgentCoreConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/agentcore/transformation.py#L47
    config: AgentCoreApi = agent  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/agentcore/transformation.py#L47
    results: list[Json]
    if input_value["scenario"] == "runtime_arn_required":
        results = [_url_result(config, {"model": model}) for model in input_value["models"]]
    elif input_value["scenario"] == "arn_region" or input_value["scenario"] == "qualifier_query":
        results = [_url_result(config, item) for item in input_value["runs"]]
    elif (
        input_value["scenario"] == "runtime_session_id"
        or input_value["scenario"] == "runtime_user_id"
        or input_value["scenario"] == "multimodal_forwarding"
    ):
        results = [_transform_result(config, item) for item in input_value["runs"]]
    elif input_value["scenario"] == "stream_body_policy":
        stream_flags: Params = {
            "has_custom_stream_wrapper": agent.has_custom_stream_wrapper,
            "supports_stream_param_in_request_body": agent.supports_stream_param_in_request_body,
        }
        return _stream_body_result(config, input_value, stream_flags)
    elif (
        input_value["scenario"] == "agentcore_bearer_signing"
        or input_value["scenario"] == "agentcore_iam_signing"
    ):
        results = [_sign_result(config, item) for item in input_value["runs"]]
    else:
        raise ValueError(input_value["scenario"])
    return {"results": results}
