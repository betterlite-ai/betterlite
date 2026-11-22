import os
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict
from uuid import UUID

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
from litellm.llms.bedrock.chat.invoke_agent.transformation import AmazonInvokeAgentConfig
from pydantic import JsonValue

Params: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
Messages: TypeAlias = list[JsonValue]

MODEL = "agent/L1RT58GYRW/MFPSBCXYTW"


class EnvironmentRun(TypedDict):
    runtime_endpoint: NotRequired[str | None]
    region_name: NotRequired[str | None]
    region: NotRequired[str | None]
    session_region: NotRequired[str | None]


class UrlRun(TypedDict):
    model: str
    api_base: NotRequired[str | None]
    optional_params: NotRequired[Params]
    environment: NotRequired[EnvironmentRun]
    uuid: str


class TransformRun(TypedDict):
    messages: Messages
    optional_params: NotRequired[Params]


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
    stream: NotRequired[bool | None]
    fake_stream: NotRequired[bool | None]
    environment: NotRequired[EnvironmentRun]


class ErrorRun(TypedDict):
    status: int
    message: str
    headers: list[tuple[str, str]]


class PolicyRun(TypedDict):
    optional_params: Params
    headers: list[tuple[str, str]]
    error: ErrorRun


class UrlScenario(TypedDict):
    scenario: Literal["agent_id_alias_path", "session_id"]
    runs: list[UrlRun]


class TransformScenario(TypedDict):
    scenario: Literal["agent_input_text"]
    runs: list[TransformRun]


class SigningScenario(TypedDict):
    scenario: Literal["agent_signing_and_capability"]
    signing: list[SignRun]
    policy: PolicyRun


Input: TypeAlias = UrlScenario | TransformScenario | SigningScenario


class ResponseLike(Protocol):
    status_code: int


class ErrorLike(Protocol):
    status_code: int
    status_code_is_synthesized: bool
    headers: Params
    response: ResponseLike


class InvokeAgentApi(Protocol):
    def get_supported_openai_params(self, model: str, /) -> list[str]: ...

    def map_openai_params(
        self, non_default: Params, params: Params, model: str, /, *, drop_params: bool
    ) -> Params: ...

    def get_complete_url(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_agent/transformation.py#L75
        self,
        base: str | None,
        key: str | None,
        model: str,
        params: Params,
        extra: Params,
        stream: bool | None = None,
        /,
    ) -> str: ...

    def sign_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_agent/transformation.py#L108
        self,
        headers: Headers,
        params: Params,
        data: Params,
        base: str,
        key: str | None = None,
        model: str | None = None,
        stream: bool | None = None,
        fake_stream: bool | None = None,
        /,
    ) -> tuple[Headers, bytes | None]: ...

    def transform_request(
        self, model: str, messages: Messages, params: Params, extra: Params, headers: Headers, /
    ) -> Params: ...

    def validate_environment(
        self, headers: Headers, model: str, messages: Messages, params: Params, extra: Params, /
    ) -> Headers: ...

    def get_error_class(self, message: str, status: int, headers: Headers, /) -> ErrorLike: ...

    def should_fake_stream(
        self, model: str | None, stream: bool | None, provider: str | None = None, /
    ) -> bool: ...


class FrozenClock:
    def __init__(self, frozen: datetime) -> None:
        self.frozen = frozen

    def __call__(self) -> datetime:
        return self.frozen


def _apply_environment(run: EnvironmentRun | None) -> None:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull
    if run is None:
        return
    runtime_endpoint = run.get("runtime_endpoint")
    if runtime_endpoint is not None:
        os.environ["AWS_BEDROCK_RUNTIME_ENDPOINT"] = runtime_endpoint
    region_name = run.get("region_name")
    if region_name is not None:
        os.environ["AWS_REGION_NAME"] = region_name
    region = run.get("region")
    if region is not None:
        os.environ["AWS_REGION"] = region
    session_region = run.get("session_region")
    if session_region is not None:
        os.environ["AWS_DEFAULT_REGION"] = session_region


def _error_kind(message: str) -> str:
    if message.startswith("Invalid model format"):
        return "invalid_model_format"
    if message.startswith("Invalid AWS region format"):
        return "invalid_region"
    if message.endswith(" is required"):
        return f"required:{message.removesuffix(' is required')}"
    if message.endswith(" cannot be a dot path segment"):
        return f"dot_segment:{message.removesuffix(' cannot be a dot path segment')}"
    raise ValueError(message)


def _url_result(config: InvokeAgentApi, run: UrlRun) -> JsonValue:
    run_params = run.get("optional_params")
    optional_params: Params = {} if run_params is None else run_params
    try:
        generated = str(UUID(run["uuid"]))
    except ValueError:
        return {"ok": False, "error": "invalid_uuid", "params": optional_params}
    _apply_environment(run.get("environment"))
    try:
        url = config.get_complete_url(run.get("api_base"), None, run["model"], optional_params, {})
    except ValueError as error:
        return {"ok": False, "error": _error_kind(str(error)), "params": optional_params}
    session_id = optional_params.get("sessionID")
    if session_id:
        return {"ok": True, "url": url, "params": optional_params}
    head, separator, tail = url.rpartition("/sessions/")
    if not separator:
        raise ValueError(url)
    segment, _, rest = tail.partition("/")
    UUID(segment)
    return {"ok": True, "url": f"{head}{separator}{generated}/{rest}", "params": optional_params}


def _transform_result(config: InvokeAgentApi, run: TransformRun) -> JsonValue:
    run_params = run.get("optional_params")
    optional_params: Params = {} if run_params is None else dict(run_params)
    try:
        payload = config.transform_request(MODEL, run["messages"], optional_params, {}, {})
    except IndexError:
        return {"ok": False, "error": "empty_messages"}
    return {"ok": True, "payload": payload}


def _sign_result(config: InvokeAgentApi, run: SignRun) -> JsonValue:
    _apply_environment(run.get("environment"))
    configured_bearer = run.get("configured_bearer")
    if configured_bearer is not None:
        os.environ["AWS_BEARER_TOKEN_BEDROCK"] = configured_bearer
    frozen = datetime.fromtimestamp(run["now"], tz=UTC).replace(tzinfo=None)
    botocore.auth.get_current_datetime = FrozenClock(frozen)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
    credentials = run["credentials"]
    run_params = run.get("optional_params")
    optional_params: Params = {} if run_params is None else dict(run_params)
    optional_params["aws_access_key_id"] = credentials["access_key_id"]
    optional_params["aws_secret_access_key"] = credentials["secret_access_key"]
    session_token = credentials.get("session_token")
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
            run.get("stream"),
            run.get("fake_stream"),
        )
    except ValueError:
        return {"ok": False, "error": "signing"}
    headers, body = signed
    return {
        "ok": True,
        "method": "POST",
        "url": run["api_base"],
        "headers": Params(headers),
        "body": "" if body is None else body.decode(),
    }


def _policy_result(config: InvokeAgentApi, case: PolicyRun) -> JsonValue:
    optional_params: Params = dict(case["optional_params"])
    request_headers: Headers = dict(case["headers"])
    error_headers: Headers = dict(case["error"]["headers"])
    supported = config.get_supported_openai_params(MODEL)
    mapped = config.map_openai_params({}, optional_params, MODEL, drop_params=False)
    validated = config.validate_environment(request_headers, MODEL, [], optional_params, {})
    error = config.get_error_class(case["error"]["message"], case["error"]["status"], error_headers)
    return {
        "supported": list[JsonValue](supported),
        "mapped": mapped,
        "should_fake_stream": [
            config.should_fake_stream(None, False),
            config.should_fake_stream(MODEL, True),
            config.should_fake_stream(MODEL, None),
        ],
        "headers": Params(validated),
        "error": {
            "status_code": error.status_code,
            "status_code_is_synthesized": error.status_code_is_synthesized,
            "headers": error.headers,
            "response_status": error.response.status_code,
        },
    }


def run(input_value: Input) -> JsonValue:
    agent = AmazonInvokeAgentConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_agent/transformation.py#L48
    config: InvokeAgentApi = agent  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_agent/transformation.py#L48
    if input_value["scenario"] == "agent_id_alias_path" or input_value["scenario"] == "session_id":
        return {"results": [_url_result(config, item) for item in input_value["runs"]]}
    if input_value["scenario"] == "agent_input_text":
        return {"results": [_transform_result(config, item) for item in input_value["runs"]]}
    if input_value["scenario"] == "agent_signing_and_capability":
        return {
            "signing": [_sign_result(config, item) for item in input_value["signing"]],
            "policy": _policy_result(config, input_value["policy"]),
        }
    raise ValueError(input_value["scenario"])
