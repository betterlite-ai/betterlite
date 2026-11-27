import sys
from collections.abc import Callable
from functools import partial
from types import ModuleType
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

import botocore.parsers  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145
import botocore.session  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145
import httpx
import litellm
from botocore.exceptions import ClientError  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L146
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.batches.handler import BedrockBatchesHandler
from litellm.llms.bedrock.embed import embedding as embedding_module
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.llms.bedrock import AwsAuthParams
from litellm.types.utils import LiteLLMBatch, LlmProviders
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

litellm.suppress_debug_info = True

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
ARGS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
CLIENT_ERROR_STATUS: Final = 300
EMPTY_RECORDS: Final[list[dict[str, JsonValue]]] = []
EMPTY_REQUESTS: Final[list[str]] = []
PROJECTED_KEYS: Final = (
    "id", "object", "endpoint", "completion_window", "status", "created_at", "in_progress_at", "completed_at", "failed_at", "cancelled_at", "expires_at",
    "finalizing_at", "expired_at", "cancelling_at", "error_file_id", "input_file_id", "output_file_id", "request_counts", "errors", "metadata", "model", "usage",
)
OPERATION_NAMES: Final[dict[str, str]] = {"StopModelInvocationJob": "stop_model_invocation_job", "GetModelInvocationJob": "get_model_invocation_job"}
PARSER: Final = botocore.parsers.create_parser("rest-json")  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145
BEDROCK: Final = botocore.session.get_session().get_service_model("bedrock")  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str]


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Input(TypedDict):
    operation: Literal["cancel", "model_job_status", "async_status"]
    batch_id: str
    credentials: Keys
    region: NotRequired[str]
    replies: NotRequired[list[Reply]]
    reply: NotRequired[Reply]
    logging: NotRequired[bool]


class ErrorBody(TypedDict, total=False):
    Error: dict[str, JsonValue]
    ResponseMetadata: dict[str, JsonValue]


CLIENT_ERROR: TypeAdapter[ErrorBody] = TypeAdapter(ErrorBody)


class SignatureConfig(Protocol):
    signature_version: str


class Provider:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token: str | None = keys.get("session_token")
        self.regions: list[str] = []

    def __call__(self, _auth: AwsAuthParams, region: str) -> "Provider":
        self.regions.append(region)
        return self


class Recorder(Logging):
    def __init__(self) -> None:
        self.events: list[dict[str, JsonValue]] = []

    @override
    def pre_call(self, *_args: JsonValue, **values: JsonValue) -> None:
        additional = ARGS.validate_python(values["additional_args"])
        self.events.append(
            {"event": "pre", "input": values["input"], "api_base": additional.get("api_base")}
        )

    @override
    def post_call(self, *_args: JsonValue, **values: JsonValue) -> None:
        self.events.append({"event": "post", "input": values["input"], "api_base": None})


def parsed_reply(reply: Reply, operation: str) -> dict[str, JsonValue]:
    model = BEDROCK.operation_model(operation)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145
    wire = {"status_code": reply["status"], "headers": reply["headers"], "body": reply["body"]}
    return PARSER.parse(wire, model.output_shape)  # type: ignore[misc, no-any-return]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145


class Service:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = replies
        self.calls: list[dict[str, JsonValue]] = []
        self.service_name = "bedrock"
        self.signature_version = ""

    def call(
        self, operation: str, identifier: JsonValue, settings: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        self.calls.append(
            {
                "service": self.service_name,
                "operation": OPERATION_NAMES[operation],
                "jobIdentifier": identifier,
                "region": settings["region_name"],
                "access_key_id": settings["aws_access_key_id"],
                "session_token": settings["aws_session_token"],
                "signature_version": self.signature_version,
            }
        )
        if not self.replies:
            raise ConnectionResetError("the fixture has no queued reply")
        reply = self.replies.pop(0)
        parsed = parsed_reply(reply, operation)
        if reply["status"] >= CLIENT_ERROR_STATUS:
            raise ClientError(parsed, operation)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L145
        return parsed


class FakeBoto3(ModuleType):
    def __init__(self, service: Service) -> None:
        super().__init__("boto3")
        self.service = service
        self.settings: dict[str, JsonValue] = {}

    def client(self, service_name: str, **settings: JsonValue) -> "FakeBoto3":
        config = cast(SignatureConfig, settings["config"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L36
        self.settings = {
            "region_name": settings["region_name"],
            "aws_access_key_id": settings["aws_access_key_id"],
            "aws_secret_access_key": settings["aws_secret_access_key"],
            "aws_session_token": settings["aws_session_token"],
        }
        self.service.service_name = service_name
        self.service.signature_version = config.signature_version
        return self

    def stop_model_invocation_job(self, **kwargs: JsonValue) -> dict[str, JsonValue]:
        return self.service.call("StopModelInvocationJob", kwargs["jobIdentifier"], self.settings)

    def get_model_invocation_job(self, **kwargs: JsonValue) -> dict[str, JsonValue]:
        return self.service.call("GetModelInvocationJob", kwargs["jobIdentifier"], self.settings)


class Fixture:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.requests: list[JsonValue] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(str(request.url))
        return httpx.Response(
            self.reply["status"],
            headers={"content-type": "application/json"},
            content=self.reply["body"].encode(),
        )

    def client(self, llm_provider: LlmProviders) -> AsyncHTTPHandler:  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L648
        transport = httpx.MockTransport(self.respond)
        return AsyncHTTPHandler(transport=transport)


def project_batch(batch: LiteLLMBatch) -> JsonValue:
    dumped = cast(dict[str, JsonValue], batch.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4320
    return {key: dumped.get(key) for key in PROJECTED_KEYS}


def client_error(body: ErrorBody) -> JsonValue:
    detail = body.get("Error")
    metadata = body.get("ResponseMetadata")
    code: JsonValue = None
    status: JsonValue = None
    if detail is not None:
        code = detail.get("Code")
    if metadata is not None:
        status = metadata.get("HTTPStatusCode")
    return {"kind": "client", "status": status, "code": code}


def async_credentials(case: Input) -> dict[str, JsonValue]:
    keys = case["credentials"]
    credentials: dict[str, JsonValue] = {
        "aws_access_key_id": keys["access_key_id"],
        "aws_secret_access_key": keys["secret_access_key"],
    }
    token = keys.get("session_token")
    if token is not None:
        credentials["aws_session_token"] = token
    return credentials


def provider_for(case: Input) -> Provider:
    provider = Provider(case["credentials"])
    BaseAWSLLM.resolve_credentials = provider  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    return provider


def settled(
    call: Callable[[], LiteLLMBatch],
    provider: Provider,
    calls: list[dict[str, JsonValue]],
    requests: list[str],
    events: list[dict[str, JsonValue]],
) -> JsonValue:
    try:
        outcome: dict[str, JsonValue] = {"batch": project_batch(call()), "error": None}
    except ClientError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L146
        body = CLIENT_ERROR.validate_python(error.response)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L146
        outcome = {"batch": None, "error": client_error(body)}
    except ValidationError:
        outcome = {"batch": None, "error": {"kind": "invalid_status", "status": None, "code": None}}
    except (AttributeError, KeyError, TypeError):
        outcome = {"batch": None, "error": {"kind": "shape", "status": None, "code": None}}
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L667
        outcome = {"batch": None, "error": {"kind": "sdk", "status": None, "code": None}}
    return {
        "batch": outcome["batch"],
        "error": outcome["error"],
        "calls": JSON.validate_python(calls),
        "resolved": JSON.validate_python(provider.regions),
        "requests": JSON.validate_python(requests),
        "log": JSON.validate_python(events),
    }


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    provider = provider_for(input_value)
    if operation == "async_status":
        reply = input_value.get("reply")
        if reply is None:
            raise ValueError("the async scenario needs one reply")
        fixture = Fixture(reply)
        embedding_module.get_async_httpx_client = fixture.client  # type: ignore[assignment, attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L648
        region = input_value.get("region")
        if region is None:
            region = "us-east-1"
        async_call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L185
            "Callable[[str, str, None], LiteLLMBatch]",
            partial(BedrockBatchesHandler._handle_async_invoke_status, **async_credentials(input_value)),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L185
        )
        call = partial(async_call, input_value["batch_id"], region, None)
        return settled(call, provider, EMPTY_RECORDS, fixture.requests, EMPTY_RECORDS)
    replies = input_value.get("replies")
    if replies is None:
        raise ValueError("the control plane scenario needs replies")
    service = Service(replies)
    sys.modules["boto3"] = FakeBoto3(service)
    recorder = Recorder() if input_value.get("logging") is True else None
    status_call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L262
        "Callable[[str, str | None, Recorder | None], LiteLLMBatch]",
        BedrockBatchesHandler._handle_model_invocation_job_status,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/batches/handler.py#L262
    )
    if operation == "cancel":
        call = partial(
            BedrockBatchesHandler.cancel_batch,
            input_value["batch_id"],
            aws_region_name=input_value.get("region"),
            logging_obj=recorder,
        )
    else:
        call = partial(status_call, input_value["batch_id"], input_value.get("region"), recorder)
    recorded = EMPTY_RECORDS if recorder is None else recorder.events
    return settled(call, provider, service.calls, EMPTY_REQUESTS, recorded)
