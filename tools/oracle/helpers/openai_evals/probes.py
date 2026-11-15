from typing import NotRequired

from litellm.types.llms.openai_evals import (
    CreateEvalRequest,
    CreateRunRequest,
    ListEvalsParams,
    ListRunsParams,
    UpdateEvalRequest,
)
from litellm.types.router import GenericLiteLLMParams as Params
from pydantic import JsonValue
from typing_extensions import TypedDict

from .config import CONFIG, Operation, ReplyOperation


class ParamsFields(TypedDict):
    api_key: NotRequired[str]
    api_base: NotRequired[str]


class ListEvalsProbeParams(ListEvalsParams):
    model: NotRequired[str]


class ListRunsProbeParams(ListRunsParams):
    order_by: NotRequired[str]


class Probe(TypedDict):
    operation: NotRequired[Operation]
    reply: NotRequired[ReplyOperation]
    eval_id: NotRequired[str]
    run_id: NotRequired[str]
    api_base: NotRequired[str | None]
    endpoint: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    litellm_params: NotRequired[ParamsFields | None]
    litellm_api_key: NotRequired[str | None]
    litellm_openai_key: NotRequired[str | None]
    env: NotRequired[dict[str, str]]
    create: NotRequired[CreateEvalRequest]
    update: NotRequired[UpdateEvalRequest]
    create_run: NotRequired[CreateRunRequest]
    list_params: NotRequired[ListEvalsProbeParams]
    list_runs_params: NotRequired[ListRunsProbeParams]
    status: NotRequired[int]
    text: NotRequired[str]


class CaseInput(TypedDict):
    probes: list[Probe]


def required(value: str | None, name: str) -> str:
    if value is None:
        raise ValueError(f"the probe needs {name}")
    return value


def headers_of(probe: Probe) -> dict[str, str]:
    headers = probe.get("headers")
    if headers is None:
        return {}
    return headers


def params_of(probe: Probe) -> Params:
    fields = probe.get("litellm_params")
    if fields is None:
        return Params()
    return Params(**fields)


def params_or_none(probe: Probe) -> Params | None:
    fields = probe.get("litellm_params")
    if fields is None:
        return None
    return Params(**fields)


def create_update_probe(probe: Probe) -> JsonValue:
    update = probe.get("update")
    if update is not None:
        try:
            url, sent, body = CONFIG.transform_update_eval_request(
                required(probe.get("eval_id"), "eval_id"), update,
                required(probe.get("api_base"), "api_base"), params_of(probe),
                headers_of(probe),
            )
        except (ValueError, TypeError, KeyError):
            return {"error": {"status": None}}
        return {"update": {"url": url, "headers": sent, "body": body}}
    create = probe.get("create")
    if create is None:
        return {"create_run": request_probe(probe)}
    try:
        body = CONFIG.transform_create_eval_request(create, params_of(probe), headers_of(probe))
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    return {"create": body}


def request_probe(probe: Probe) -> JsonValue:
    create_run = probe.get("create_run")
    if create_run is not None:
        try:
            url, body = CONFIG.transform_create_run_request(
                required(probe.get("eval_id"), "eval_id"), create_run, params_of(probe),
                headers_of(probe),
            )
        except (ValueError, TypeError, KeyError):
            return {"error": {"status": None}}
        return {"url": url, "body": body}
    eval_params = probe.get("list_params")
    if eval_params is not None:
        try:
            url, query = CONFIG.transform_list_evals_request(
                eval_params, params_of(probe), headers_of(probe)
            )
        except (ValueError, TypeError, KeyError):
            return {"error": {"status": None}}
        return {"url": url, "query": query}
    run_params = probe.get("list_runs_params")
    if run_params is None:
        raise ValueError("a request probe needs create_run, list_params or list_runs_params")
    try:
        url, query = CONFIG.transform_list_runs_request(
            required(probe.get("eval_id"), "eval_id"), run_params, params_of(probe),
            headers_of(probe),
        )
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    return {"url": url, "query": query}


def id_probe(probe: Probe) -> JsonValue:
    operation = required(probe.get("operation"), "operation")
    run_id = probe.get("run_id")
    if run_id is None:
        return eval_id_probe(operation, probe)
    return run_id_probe(operation, run_id, probe)


def eval_id_probe(operation: str, probe: Probe) -> JsonValue:
    eval_id = required(probe.get("eval_id"), "eval_id")
    api_base = required(probe.get("api_base"), "api_base")
    params = params_of(probe)
    headers = headers_of(probe)
    try:
        if operation == "get":
            url, sent = CONFIG.transform_get_eval_request(eval_id, api_base, params, headers)
            result: JsonValue = {"url": url, "headers": sent}
        elif operation == "delete":
            url, sent = CONFIG.transform_delete_eval_request(eval_id, api_base, params, headers)
            result = {"url": url, "headers": sent}
        else:
            url, sent, body = CONFIG.transform_cancel_eval_request(
                eval_id, api_base, params, headers
            )
            result = {"url": url, "headers": sent, "body": body}
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
    else:
        return result


def run_id_probe(operation: str, run_id: str, probe: Probe) -> JsonValue:
    params = params_of(probe)
    headers = headers_of(probe)
    try:
        match operation:
            case "get":
                url, sent = CONFIG.transform_get_run_request(
                    required(probe.get("eval_id"), "eval_id"), run_id,
                    required(probe.get("api_base"), "api_base"), params, headers,
                )
                return {"url": url, "headers": sent}
            case "delete":
                url, sent, body = CONFIG.transform_delete_run_request(
                    required(probe.get("eval_id"), "eval_id"), run_id,
                    required(probe.get("api_base"), "api_base"), params, headers,
                )
                return {"url": url, "headers": sent, "body": body}
            case _:
                url, sent, body = CONFIG.transform_cancel_run_request(
                    required(probe.get("eval_id"), "eval_id"), run_id,
                    required(probe.get("api_base"), "api_base"), params, headers,
                )
                return {"url": url, "headers": sent, "body": body}
    except (ValueError, TypeError, KeyError):
        return {"error": {"status": None}}
