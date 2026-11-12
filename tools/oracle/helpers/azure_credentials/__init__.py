from collections.abc import Callable
from typing import cast

import httpx
from litellm.llms.azure import common_utils
from litellm.llms.azure.common_utils import (
    get_azure_ad_token,
    get_azure_request_auth_headers,
    process_azure_headers,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L66
    redact_azure_auth_headers,
)
from litellm.types.router import GenericLiteLLMParams

from azure_credentials.fixtures import (
    STATE,
    HeaderStep,
    Input,
    Json,
    ResponseStep,
    Script,
    ScriptedProvider,
    TokenStep,
    session,
)


def request_params(step: TokenStep) -> GenericLiteLLMParams:
    values: dict[str, str | Callable[[], str]] = {}
    if "params" in step:
        values.update(step["params"])
    if "provider" in step:
        values["azure_ad_token_provider"] = ScriptedProvider(step["provider"])
    return cast(GenericLiteLLMParams, values)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L300


def resolve_outcome(request: GenericLiteLLMParams) -> dict[str, Json]:
    try:
        token = get_azure_ad_token(request)
    except (TypeError, KeyError, ValueError, RuntimeError) as error:
        return {"kind": "error", "class": type(error).__name__}
    if token is None:
        return {"kind": "absent"}
    return {"kind": "token", "token": token}


def cache_slot(scope: str) -> Json:
    provider = common_utils._cached_azure_ad_token_refresh_provider(scope)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L100
    key = id(provider)
    if key in STATE.slots:
        return STATE.slots[key]
    slot = len(STATE.slots)
    STATE.slots[key] = slot
    STATE.held.append(provider)
    return slot


def cache_report(scopes: list[str]) -> Json:
    STATE.slots.clear()
    STATE.held.clear()
    common_utils._cached_azure_ad_token_refresh_provider.cache_clear()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L99-L101
    slots: list[Json] = [cache_slot(scope) for scope in scopes]
    revisit = cache_slot(scopes[0])
    common_utils._cached_azure_ad_token_refresh_provider.cache_clear()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L99-L101
    return {"slots": slots, "revisit": revisit}


def token_report(step: TokenStep) -> Json:
    environment: dict[str, str] = {}
    sources: dict[str, Script] = {}
    refresh = False
    if "environment" in step:
        environment = step["environment"]
    if "sources" in step:
        sources = step["sources"]
    if "refresh" in step:
        refresh = step["refresh"]
    with session(environment, sources, refresh=refresh):
        outcome = resolve_outcome(request_params(step))
        scopes = step["cache"]
        cache: Json = cache_report(scopes) if scopes else None
        return {
            "calls": list(STATE.calls),
            "provider_calls": STATE.provider_calls,
            "outcome": outcome,
            "cache": cache,
        }


def request_header_report(step: HeaderStep) -> Json:
    with session({}, {}, refresh=False):
        headers: dict[str, str] = {}
        if "headers" in step:
            headers = step["headers"]
        client: dict[str, str | Callable[[], str]] = {}
        if "client" in step:
            client.update(step["client"])
        if "provider" in step:
            client["azure_ad_token_provider"] = ScriptedProvider(step["provider"])
        try:
            output = get_azure_request_auth_headers(headers, client)
        except (TypeError, KeyError, ValueError, RuntimeError) as error:
            return {
                "kind": "error",
                "class": type(error).__name__,
                "provider_calls": STATE.provider_calls,
            }
        return {
            "kind": "headers",
            "headers": dict(output),
            "provider_calls": STATE.provider_calls,
        }


def redact_report(step: HeaderStep) -> Json:
    headers: dict[str, str] = {}
    if "headers" in step:
        headers = step["headers"]
    return {"kind": "headers", "headers": dict(redact_azure_auth_headers(headers))}


def response_report(step: ResponseStep) -> Json:
    headers: dict[str, str] | httpx.Headers = step["headers"]
    if step["source"] == "httpx":
        headers = httpx.Headers(headers)
    processed = cast(dict[str, Json], process_azure_headers(headers))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L66
    return {"kind": "headers", "headers": processed}


def run(argument: Input) -> Json:
    if "token" in argument:
        return {"steps": [token_report(step) for step in argument["token"]]}
    if "request_headers" in argument:
        return {"steps": [request_header_report(step) for step in argument["request_headers"]]}
    if "redact_headers" in argument:
        return {"steps": [redact_report(step) for step in argument["redact_headers"]]}
    if "response_headers" in argument:
        return {"steps": [response_report(step) for step in argument["response_headers"]]}
    raise ValueError("expected one of token, request_headers, redact_headers or response_headers")
