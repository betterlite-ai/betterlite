import asyncio
from collections.abc import Coroutine
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import httpx
import litellm
import openai
import pydantic
from litellm.llms.azure.common_utils import BaseAzureLLM
from litellm.llms.azure.fine_tuning.handler import AzureOpenAIFineTuningAPI
from openai import DEFAULT_TIMEOUT, AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue, TypeAdapter

ENDPOINT = "https://fixture.test"
KEY = "supplied-key"
VERSION = "2024-10-21"

JSON = TypeAdapter(JsonValue)
WIRE = TypeAdapter(dict[str, JsonValue])
API_VERSION = TypeAdapter(str | None)


class ErrorBody(TypedDict):
    message: str


MESSAGE = TypeAdapter(ErrorBody)

SdkClient: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI
ModelResult: TypeAlias = pydantic.BaseModel | Coroutine[object, object, pydantic.BaseModel]
JsonObject: TypeAlias = dict[str, JsonValue]


class Values(TypedDict):
    api_key: str | None
    api_base: str | None
    api_version: str | None
    timeout: float
    max_retries: int | None
    organization: str | None


class Reply(TypedDict):
    status: int
    body: JsonValue


class Step(TypedDict):
    action: Literal["create", "list", "retrieve", "cancel", "project"]
    args: Values
    is_async: NotRequired[bool]
    client: NotRequired[str]
    builder: NotRequired[str]
    data: NotRequired[JsonObject]
    job_id: NotRequired[str]
    after: NotRequired[str]
    limit: NotRequired[int]
    reply: NotRequired[Reply]


class Input(TypedDict):
    steps: list[Step]


class State:
    def __init__(self) -> None:
        self.builder = "default"
        self.mode_client: SdkClient | None = None
        self.reply: Reply | None = None
        self.calls: list[JsonValue] = []
        self.supplied_calls: list[JsonValue] = []


STATE = State()


def record_call(request: httpx.Request, log: list[JsonValue]) -> None:
    path = request.url.path
    if path.endswith("/cancel"):
        log.append({"cancel": {"fine_tuning_job_id": path.split("/")[-2]}})
    elif request.method == "POST":
        log.append({"create": WIRE.validate_json(request.content)})
    elif path.endswith("/jobs"):
        after = request.url.params.get_list("after")
        limit = request.url.params.get_list("limit")
        listed: JsonObject = {
            "after": after[0] if after else None,
            "limit": int(limit[0]) if limit else None,
        }
        log.append({"list": listed})
    else:
        log.append({"retrieve": {"fine_tuning_job_id": path.split("/")[-1]}})


def response_for(reply: Reply | None) -> httpx.Response:
    if reply is None:
        empty: JsonObject = {}
        return httpx.Response(200, json=empty)
    return httpx.Response(reply["status"], json=reply["body"])


class Record:
    def __init__(self, log: list[JsonValue]) -> None:
        self.log = log

    def __call__(self, request: httpx.Request) -> httpx.Response:
        record_call(request, self.log)
        return response_for(STATE.reply)


REAL_AZURE_CLIENT = BaseAzureLLM.get_azure_openai_client  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489


def build_client(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
    self: BaseAzureLLM,
    *,
    api_key: str | None = None,
    api_base: str | None = None,
    api_version: str | None = None,
    client: SdkClient | None = None,
    litellm_params: dict[str, JsonValue] | None = None,
    _is_async: bool = False,
    model: str | None = None,
) -> SdkClient | None:
    if STATE.builder == "none":
        return None
    if STATE.builder == "wrong_async":
        return STATE.mode_client
    return REAL_AZURE_CLIENT(
        self, api_key, api_base, api_version, client, litellm_params, _is_async, model
    )


BaseAzureLLM.get_azure_openai_client = build_client  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/fine_tuning/handler.py#L197-L204


def make_client(kind: str, log: list[JsonValue]) -> SdkClient:
    wire = httpx.MockTransport(Record(log))
    if kind == "openai":
        return OpenAI(
            api_key=KEY, base_url=ENDPOINT, max_retries=0, http_client=httpx.Client(transport=wire)
        )
    if kind == "async_openai":
        http_client = httpx.AsyncClient(transport=wire)
        return AsyncOpenAI(api_key=KEY, base_url=ENDPOINT, max_retries=0, http_client=http_client)
    if kind == "azure_openai":
        return AzureOpenAI(
            api_key=KEY, api_version=VERSION, azure_endpoint=ENDPOINT,
            max_retries=0, http_client=httpx.Client(transport=wire),
        )
    if kind == "async_azure_openai":
        return AsyncAzureOpenAI(
            api_key=KEY, api_version=VERSION, azure_endpoint=ENDPOINT,
            max_retries=0, http_client=httpx.AsyncClient(transport=wire),
        )
    raise ValueError(f"unknown supplied client {kind}")


def settle(result: ModelResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case object():
            return JSON.validate_json(result.model_dump_json())


def project(step: Step, supplied: SdkClient | None, *, is_async: bool) -> JsonValue:
    api = AzureOpenAIFineTuningAPI()
    built = api.get_openai_client(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/fine_tuning/handler.py#L181-L204
        **step["args"], client=supplied, _is_async=is_async
    )
    if built is None:
        raise ValueError("the client builder returned no client")
    return {
        "class": type(built).__name__,
        "api_key": built.api_key,
        "max_retries": built.max_retries,
        "organization": built.organization,
        "timeout_default": built.timeout == DEFAULT_TIMEOUT,
        "base_url": str(built.base_url),
        "api_version": API_VERSION.validate_python(built.default_query.get("api-version")),
    }


def create_job(step: Step, supplied: SdkClient | None, *, is_async: bool) -> JsonValue:
    data = step.get("data")
    if data is None:
        raise ValueError("a create step carries no data")
    api = AzureOpenAIFineTuningAPI()
    created = api.create_fine_tuning_job(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/fine_tuning/handler.py#L58-L99
        _is_async=is_async, create_fine_tuning_job_data=data, client=supplied, **step["args"]
    )
    return settle(created)


def list_jobs(step: Step, supplied: SdkClient | None, *, is_async: bool) -> JsonValue:
    api = AzureOpenAIFineTuningAPI()
    listed = cast(ModelResult, api.list_fine_tuning_jobs(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/fine_tuning/handler.py#L201-L241
        _is_async=is_async, client=supplied, after=step.get("after"), limit=step.get("limit"),
        **step["args"],
    ))
    return settle(listed)


def job_action(
    step: Step, supplied: SdkClient | None, *, is_async: bool, cancel: bool
) -> JsonValue:
    api = AzureOpenAIFineTuningAPI()
    job_id = step.get("job_id")
    if job_id is None:
        raise ValueError("a job step carries no job_id")
    if cancel:
        return settle(
            api.cancel_fine_tuning_job(
                _is_async=is_async, fine_tuning_job_id=job_id, client=supplied, **step["args"]
            )
        )
    return settle(
        api.retrieve_fine_tuning_job(
            _is_async=is_async, fine_tuning_job_id=job_id, client=supplied, **step["args"]
        )
    )


def act(step: Step) -> JsonValue:
    action = step["action"]
    is_async = bool(step.get("is_async"))
    kind = step.get("client")
    supplied = None if kind is None else make_client(kind, STATE.supplied_calls)
    if action == "project":
        return project(step, supplied, is_async=is_async)
    if action == "create":
        return create_job(step, supplied, is_async=is_async)
    if action == "list":
        return list_jobs(step, supplied, is_async=is_async)
    return job_action(step, supplied, is_async=is_async, cancel=action == "cancel")


def status_error(error: openai.APIStatusError) -> JsonObject:
    return {
        "class": "APIStatusError",
        "status": error.status_code,
        "type": error.type,
        "param": error.param,
        "code": error.code,
        "message": MESSAGE.validate_python(error.body)["message"],
    }


def outcome(step: Step) -> JsonValue:
    try:
        return {"returned": act(step)}
    except pydantic.ValidationError:
        return {"raised": {"class": "ValidationError"}}
    except ValueError as error:
        return {"raised": {"class": "ValueError", "message": str(error)}}
    except TypeError:
        return {"raised": {"class": "TypeError"}}
    except openai.APIStatusError as error:
        return {"raised": status_error(error)}


def run_step(step: Step) -> JsonObject:
    STATE.calls = []
    STATE.supplied_calls = []
    STATE.reply = step.get("reply")
    builder = step.get("builder")
    STATE.builder = "default" if builder is None else builder
    STATE.mode_client = (
        make_client("azure_openai", STATE.calls) if STATE.builder == "wrong_async" else None
    )
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(Record(STATE.calls)))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(Record(STATE.calls)))
    reported = outcome(step)
    return {
        "data": step.get("data"),
        "outcome": reported,
        "calls": STATE.calls,
        "supplied_calls": STATE.supplied_calls,
    }


def run(input_value: Input) -> JsonValue:
    steps: list[JsonValue] = [run_step(step) for step in input_value["steps"]]
    return {"steps": steps}
