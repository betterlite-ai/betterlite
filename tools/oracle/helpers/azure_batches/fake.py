from typing import Literal

import httpx
import openai
from litellm.llms.azure.common_utils import BaseAzureLLM
from openai.pagination import SyncCursorPage
from openai.resources.batches import AsyncBatches, Batches
from openai.types.batch import Batch
from pydantic import JsonValue

from .contracts import ENDPOINT, ENVELOPE, ERROR_STATUS, JsonObject, Reply, SdkClient


class State:
    def __init__(self) -> None:
        self.builder: Literal["default", "none"] = "default"
        self.reply: Reply | None = None
        self.calls: list[JsonValue] = []


STATE = State()


def planned() -> Reply:
    reply = STATE.reply
    if reply is None:
        raise ValueError("a call step carries no reply")
    return reply


def refusal(reply: Reply) -> openai.APIStatusError:
    payload = ENVELOPE.validate_python(reply["body"])["error"]
    request = httpx.Request("POST", ENDPOINT)
    response = httpx.Response(reply["status"], request=request)
    return openai.APIStatusError(payload["message"], response=response, body=payload)


def answered_batch() -> Batch:
    reply = planned()
    if reply["status"] >= ERROR_STATUS:
        raise refusal(reply)
    return Batch.model_validate(reply["body"])


def answered_page() -> SyncCursorPage[Batch]:
    reply = planned()
    if reply["status"] >= ERROR_STATUS:
        raise refusal(reply)
    return SyncCursorPage[Batch].model_validate(reply["body"])


def record(method: str, kwargs: dict[str, JsonValue]) -> None:
    call: JsonObject = {method: kwargs}
    STATE.calls.append(call)


def create_call(_client: Batches | AsyncBatches, **kwargs: JsonValue) -> Batch:
    record("create", kwargs)
    return answered_batch()


def retrieve_call(_client: Batches | AsyncBatches, **kwargs: JsonValue) -> Batch:
    record("retrieve", kwargs)
    return answered_batch()


def cancel_call(_client: Batches | AsyncBatches, **kwargs: JsonValue) -> Batch:
    record("cancel", kwargs)
    return answered_batch()


def list_call(_client: Batches | AsyncBatches, **kwargs: JsonValue) -> SyncCursorPage[Batch]:
    record("list", kwargs)
    return answered_page()


async def acreate_call(_client: AsyncBatches, **kwargs: JsonValue) -> Batch:
    record("create", kwargs)
    return answered_batch()


async def aretrieve_call(_client: AsyncBatches, **kwargs: JsonValue) -> Batch:
    record("retrieve", kwargs)
    return answered_batch()


async def acancel_call(_client: AsyncBatches, **kwargs: JsonValue) -> Batch:
    record("cancel", kwargs)
    return answered_batch()


async def alist_call(_client: AsyncBatches, **kwargs: JsonValue) -> SyncCursorPage[Batch]:
    record("list", kwargs)
    return answered_page()


Batches.create = create_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L73
Batches.retrieve = retrieve_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L115
Batches.cancel = cancel_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L163
Batches.list = list_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L207
AsyncBatches.create = acreate_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L39
AsyncBatches.retrieve = aretrieve_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L81
AsyncBatches.cancel = acancel_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L123
AsyncBatches.list = alist_call  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L172

REAL_AZURE_CLIENT = BaseAzureLLM.get_azure_openai_client  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489


def build_client(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
    self: BaseAzureLLM,
    *,
    api_key: str | None,
    api_base: str | None,
    api_version: str | None = None,
    client: SdkClient | None = None,
    litellm_params: dict[str, JsonValue] | None = None,
    _is_async: bool = False,
    model: str | None = None,
) -> SdkClient | None:
    if STATE.builder == "none":
        return None
    return REAL_AZURE_CLIENT(
        self, api_key, api_base, api_version, client, litellm_params, _is_async, model
    )


BaseAzureLLM.get_azure_openai_client = build_client  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L54-L61
