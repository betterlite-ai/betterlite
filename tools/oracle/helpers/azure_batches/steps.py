import asyncio
from collections.abc import Coroutine
from typing import Literal, TypeAlias, cast

import openai
import pydantic
from litellm.llms.azure.batches.handler import AzureBatchesAPI
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue

from .contracts import (
    CLIENT_MISSING,
    ENDPOINT,
    JSON,
    KEY,
    PAYLOAD,
    VERSION,
    VERSION_QUERY,
    JsonObject,
    SdkClient,
    Step,
)
from .fake import STATE

ModelResult: TypeAlias = pydantic.BaseModel | Coroutine[object, object, pydantic.BaseModel]


def make_client(kind: str) -> SdkClient:
    if kind == "openai":
        return OpenAI(api_key=KEY, base_url=ENDPOINT, max_retries=0)
    if kind == "async_openai":
        return AsyncOpenAI(api_key=KEY, base_url=ENDPOINT, max_retries=0)
    if kind == "azure_openai":
        return AzureOpenAI(
            api_key=KEY, api_version=VERSION, azure_endpoint=ENDPOINT, max_retries=0
        )
    if kind == "async_azure_openai":
        return AsyncAzureOpenAI(
            api_key=KEY, api_version=VERSION, azure_endpoint=ENDPOINT, max_retries=0
        )
    raise ValueError(f"unknown supplied client {kind}")


def settle(result: ModelResult) -> pydantic.BaseModel:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                return runner.run(result)
        case object():
            return result


def project(step: Step, supplied: SdkClient | None, *, is_async: bool) -> JsonValue:
    args = step["args"]
    built = AzureBatchesAPI().get_azure_openai_client(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L54-L61
        api_key=args["api_key"],
        api_base=args["api_base"],
        api_version=args["api_version"],
        client=supplied,
        litellm_params=step.get("params"),
        _is_async=is_async,
    )
    if built is None:
        raise ValueError(CLIENT_MISSING)
    return {
        "class": type(built).__name__,
        "api_key": built.api_key,
        "max_retries": built.max_retries,
        "organization": built.organization,
        "timeout_default": built.timeout == openai.DEFAULT_TIMEOUT,
        "base_url": str(built.base_url),
        "api_version": VERSION_QUERY.validate_python(built.default_query.get("api-version")),
    }


def request_step(
    step: Step,
    supplied: SdkClient | None,
    *,
    is_async: bool,
    action: Literal["create", "retrieve", "cancel"],
) -> JsonValue:
    api = AzureBatchesAPI()
    args = step["args"]
    if action == "create":
        data = step.get("data")
        if data is None:
            raise ValueError("a create step carries no data")
        created = api.create_batch(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L42-L74
            _is_async=is_async,
            create_batch_data=data,
            api_key=args["api_key"],
            api_base=args["api_base"],
            api_version=args["api_version"],
            timeout=args["timeout"],
            max_retries=args["max_retries"],
            client=supplied,
            litellm_params=step.get("params"),
        )
        return JSON.validate_json(settle(created).model_dump_json())
    query = step.get("query")
    if query is None:
        raise ValueError(f"a {action} step carries no query")
    if action == "cancel":
        cancelled = cast(ModelResult, api.cancel_batch(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L126-L164
            _is_async=is_async,
            cancel_batch_data=query,
            api_key=args["api_key"],
            api_base=args["api_base"],
            api_version=args["api_version"],
            timeout=args["timeout"],
            max_retries=args["max_retries"],
            client=supplied,
            litellm_params=step.get("params"),
        ))
        return JSON.validate_json(settle(cancelled).model_dump_json())
    retrieved = cast(ModelResult, api.retrieve_batch(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L84-L116
        _is_async=is_async,
        retrieve_batch_data=query,
        api_key=args["api_key"],
        api_base=args["api_base"],
        api_version=args["api_version"],
        timeout=args["timeout"],
        max_retries=args["max_retries"],
        client=supplied,
        litellm_params=step.get("params"),
    ))
    return JSON.validate_json(settle(retrieved).model_dump_json())


def list_step(step: Step, supplied: SdkClient | None, *, is_async: bool) -> JsonValue:
    args = step["args"]
    listed = cast(ModelResult, AzureBatchesAPI().list_batches(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L175-L208
        _is_async=is_async,
        api_key=args["api_key"],
        api_base=args["api_base"],
        api_version=args["api_version"],
        timeout=args["timeout"],
        max_retries=args["max_retries"],
        after=step.get("after"),
        limit=step.get("limit"),
        client=supplied,
        litellm_params=step.get("params"),
    ))
    return JSON.validate_json(settle(listed).model_dump_json())


def act(step: Step) -> JsonValue:
    action = step["action"]
    is_async = bool(step.get("is_async"))
    kind = step.get("client")
    supplied = None if kind is None else make_client(kind)
    if action == "project":
        return project(step, supplied, is_async=is_async)
    if action == "list":
        return list_step(step, supplied, is_async=is_async)
    return request_step(step, supplied, is_async=is_async, action=action)


def status_error(error: openai.APIStatusError) -> JsonValue:
    body = PAYLOAD.validate_python(error.body)
    return {
        "class": "APIStatusError",
        "status": error.status_code,
        "type": error.type,
        "param": error.param,
        "code": error.code,
        "message": body["message"],
    }


def outcome(step: Step) -> JsonValue:
    try:
        return {"returned": act(step)}
    except pydantic.ValidationError:
        return {"raised": {"class": "ValidationError"}}
    except ValueError as error:
        return {"raised": {"class": "ValueError", "message": str(error)}}
    except openai.APIStatusError as error:
        return {"raised": status_error(error)}


def run_step(step: Step) -> JsonObject:
    STATE.calls = []
    STATE.reply = step.get("reply")
    builder = step.get("builder")
    STATE.builder = "default" if builder is None else builder
    report = outcome(step)
    return {"outcome": report, "calls": STATE.calls}
