import asyncio
import os
from collections.abc import Coroutine, Generator
from contextlib import contextmanager
from typing import Literal, cast

import httpx
import litellm
import openai
from pydantic import JsonValue

from azure_assistant_definitions.sdk import (
    ASSISTANT,
    ASYNC_GET_CLIENT,
    CREATE,
    DEFAULT_TIMEOUT,
    DELETE,
    DELETED,
    GET_CLIENT,
    JSON,
    LIST,
    PAGE,
    Call,
    Input,
    Reply,
    Result,
    SdkClient,
    Settled,
    Step,
    Supplied,
    SuppliedMark,
    Values,
    Wire,
)


@contextmanager
def session(step: Step, canned: str) -> Generator[Wire, None, None]:
    reply: Reply | None = None
    if "reply" in step:
        reply = step["reply"]
    wire = Wire(canned, reply)
    environment: dict[str, str] = {}
    if "environment" in step:
        environment = step["environment"]
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(environment)
    sync_client = httpx.Client(transport=httpx.MockTransport(wire))
    async_client = httpx.AsyncClient(transport=httpx.MockTransport(wire))
    litellm.client_session = sync_client
    litellm.aclient_session = async_client
    try:
        yield wire
    finally:
        litellm.client_session = None
        litellm.aclient_session = None
        sync_client.close()
        asyncio.run(async_client.aclose())
        os.environ.clear()
        os.environ.update(saved)


@contextmanager
def reported(wire: Wire) -> Generator[None, None, None]:
    try:
        yield
    except openai.APIStatusError as error:
        wire.error = {"kind": "status", "status": error.status_code}
    except openai.OpenAIError:
        wire.error = {"kind": "configuration"}
    except ValueError:
        wire.error = {"kind": "value"}


def report(wire: Wire) -> JsonValue:
    values: dict[str, JsonValue] = {"requests": wire.seen}
    if wire.error is None:
        values["kind"] = wire.kind
        values["result"] = wire.result
    else:
        values["error"] = wire.error
    return values


def client_report(client: SdkClient, kind: str, source: str) -> JsonValue:
    timeout = client.timeout
    read_seconds: JsonValue = None
    match timeout:
        case float() | int() as seconds:
            read_seconds = float(seconds)
        case httpx.Timeout(read=float() as read):
            read_seconds = float(read)
        case None | httpx.Timeout():
            read_seconds = None
    values: dict[str, JsonValue] = {
        "kind": kind,
        "source": source,
        "base_url": str(client.base_url).rstrip("/"),
        "max_retries": client.max_retries,
        "timeout": read_seconds,
    }
    return values


def values_of(step: Step) -> Values:
    values = Values(None, None, None, None, DEFAULT_TIMEOUT, None, {}, None)
    if "api_key" in step:
        values.api_key = step["api_key"]
    if "api_base" in step:
        values.api_base = step["api_base"]
    if "api_version" in step:
        values.api_version = step["api_version"]
    if "azure_ad_token" in step:
        values.azure_ad_token = step["azure_ad_token"]
    if "timeout" in step:
        values.timeout = float(step["timeout"])
    if "max_retries" in step:
        values.max_retries = step["max_retries"]
    if "flag" in step:
        values.flag = step["flag"]
    if "params" in step:
        values.litellm_params = dict(step["params"])
    return values


def settle(result: Result) -> Settled:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return {
                "kind": "async",
                "result": JSON.validate_json(settled.model_dump_json(exclude_none=True)),
            }
        case object():
            return {
                "kind": "sync",
                "result": JSON.validate_json(result.model_dump_json(exclude_none=True)),
            }


def record_reply(wire: Wire, result: Result) -> None:
    settled = settle(result)
    wire.kind = settled["kind"]
    wire.result = settled["result"]


def client_params(values: Values) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {
        "api_key": values.api_key,
        "api_base": values.api_base,
        "api_version": values.api_version,
        "azure_ad_token": values.azure_ad_token,
        "timeout": values.timeout,
        "max_retries": values.max_retries,
        "client": None,
        "litellm_params": values.litellm_params,
    }
    return params


def run_call(step: Step, canned: str, call: Call, params: dict[str, JsonValue]) -> JsonValue:
    with session(step, canned) as wire, reported(wire):
        record_reply(wire, call(**params))
    return report(wire)


def list_step(step: Step) -> JsonValue:
    values = values_of(step)
    params = client_params(values)
    params["aget_assistants"] = values.flag
    return run_call(step, PAGE, LIST, params)


def mutate_step(step: Step, variant: Literal["create", "delete"]) -> JsonValue:
    values = values_of(step)
    params = client_params(values)
    if variant == "create":
        params["create_assistant_data"] = step.get("data")
        params["async_create_assistants"] = values.flag
        return run_call(step, ASSISTANT, CREATE, params)
    params["assistant_id"] = step.get("assistant_id")
    params["async_delete_assistants"] = values.flag
    return run_call(step, DELETED, DELETE, params)


def client_step(step: Step) -> JsonValue:
    values = values_of(step)
    supplied: Supplied | None = None
    if "supplied" in step:
        supplied = step["supplied"]
    kind: Literal["sync", "async"] = "sync"
    if "kind" in step:
        kind = step["kind"]
    mark: SuppliedMark | None = None
    source = "constructed"
    if supplied is not None:
        kind = supplied["kind"]
        mark = SuppliedMark(kind)
        source = "supplied"
    supplied_client: SdkClient | None = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L51-L52
        "SdkClient | None",
        mark,
    )
    shown: JsonValue = None
    with session(step, PAGE) as wire, reported(wire):
        if kind == "async":
            client: SdkClient = ASYNC_GET_CLIENT(
                api_key=values.api_key,
                api_base=values.api_base,
                api_version=values.api_version,
                azure_ad_token=values.azure_ad_token,
                timeout=values.timeout,
                max_retries=values.max_retries,
                client=supplied_client,
                litellm_params=values.litellm_params,
            )
        else:
            client = GET_CLIENT(
                api_key=values.api_key,
                api_base=values.api_base,
                api_version=values.api_version,
                azure_ad_token=values.azure_ad_token,
                timeout=values.timeout,
                max_retries=values.max_retries,
                client=supplied_client,
                litellm_params=values.litellm_params,
            )
        shown = client_report(client, kind, source)
    if wire.error is not None:
        failed: dict[str, JsonValue] = {
            "kind": kind,
            "source": source,
            "error": wire.error,
        }
        return failed
    return shown


def run(argument: Input) -> JsonValue:
    steps: list[JsonValue] = []
    if "list_assistants" in argument:
        steps = [list_step(step) for step in argument["list_assistants"]]
    elif "create_assistant" in argument:
        steps = [mutate_step(step, "create") for step in argument["create_assistant"]]
    elif "delete_assistant" in argument:
        steps = [mutate_step(step, "delete") for step in argument["delete_assistant"]]
    elif "client_arguments" in argument:
        steps = [client_step(step) for step in argument["client_arguments"]]
    else:
        raise ValueError("expected one azure assistant definitions variant")
    reports: dict[str, JsonValue] = {"steps": steps}
    return reports
