from collections.abc import Callable
from typing import cast

import httpx
import litellm
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
    ClientParams,
    HeaderStep,
    Input,
    Json,
    ResponseStep,
    SourceScript,
    TokenStep,
    provider_from,
    session,
)


def environment_of(step: TokenStep) -> dict[str, str]:
    if "environment" in step:
        return step["environment"]
    return {}


def sources_of(step: TokenStep) -> dict[str, SourceScript]:
    if "sources" in step:
        return step["sources"]
    return {}


def refresh_of(step: TokenStep) -> bool:
    if "refresh" in step:
        return step["refresh"]
    return False


def request_params(step: TokenStep) -> GenericLiteLLMParams:
    values: dict[str, str | Callable[[], str]] = {}
    if "params" in step:
        params = step["params"]
        if "azure_ad_token" in params:
            values["azure_ad_token"] = params["azure_ad_token"]
        if "tenant_id" in params:
            values["tenant_id"] = params["tenant_id"]
        if "client_id" in params:
            values["client_id"] = params["client_id"]
        if "client_secret" in params:
            values["client_secret"] = params["client_secret"]
        if "azure_scope" in params:
            values["azure_scope"] = params["azure_scope"]
        if "azure_username" in params:
            values["azure_username"] = params["azure_username"]
        if "azure_password" in params:
            values["azure_password"] = params["azure_password"]
    if "provider" in step:
        values["azure_ad_token_provider"] = provider_from(step["provider"])
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
    with session(environment_of(step), sources_of(step)):
        litellm.enable_azure_ad_token_refresh = refresh_of(step)
        outcome = resolve_outcome(request_params(step))
        scopes = step["cache"]
        cache: Json = cache_report(scopes) if scopes else None
        return {
            "calls": list(STATE.calls),
            "provider_calls": STATE.provider_calls,
            "outcome": outcome,
            "cache": cache,
        }


def headers_of(step: HeaderStep) -> dict[str, str]:
    if "headers" in step:
        return step["headers"]
    return {}


def add_client(client: dict[str, str | Callable[[], str]], values: ClientParams) -> None:
    if "azure_ad_token" in values:
        client["azure_ad_token"] = values["azure_ad_token"]
    if "api_key" in values:
        client["api_key"] = values["api_key"]


def request_header_report(step: HeaderStep) -> Json:
    with session({}, {}):
        headers = headers_of(step)
        client: dict[str, str | Callable[[], str]] = {}
        if "client" in step:
            add_client(client, step["client"])
        if "provider" in step:
            client["azure_ad_token_provider"] = provider_from(step["provider"])
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
    headers = headers_of(step)
    return {"kind": "headers", "headers": dict(redact_azure_auth_headers(headers))}


def response_report(step: ResponseStep) -> Json:
    headers = dict(step["headers"])
    if step["source"] == "httpx":
        processed = cast(dict[str, Json], process_azure_headers(httpx.Headers(headers)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L66
        return {"kind": "headers", "headers": processed}
    processed = cast(dict[str, Json], process_azure_headers(headers))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L66
    return {"kind": "headers", "headers": processed}


def run(argument: Input) -> Json:
    steps: list[Json] = []
    if "token" in argument:
        steps.extend([token_report(step) for step in argument["token"]["steps"]])
    elif "request_headers" in argument:
        steps.extend(
            [request_header_report(step) for step in argument["request_headers"]["steps"]]
        )
    elif "redact_headers" in argument:
        steps.extend([redact_report(step) for step in argument["redact_headers"]["steps"]])
    elif "response_headers" in argument:
        steps.extend([response_report(step) for step in argument["response_headers"]["steps"]])
    else:
        raise ValueError(
            "expected one of token, request_headers, redact_headers or response_headers"
        )
    return {"steps": steps}
