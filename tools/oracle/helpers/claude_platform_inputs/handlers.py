import base64
from collections.abc import Callable
from functools import partial
from typing import cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L433
import litellm
from litellm.exceptions import AuthenticationError
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.claude_platform.common_utils import (
    BedrockClaudePlatformMixin,
    filter_claude_platform_request_body,
    resolve_unsupported_override,
    strip_claude_platform_route,
)

from .capture import captured_warnings
from .patching import CredentialProvider, FrozenClock, set_environment
from .types import (
    EMPTY_HEADER_MAP,
    EMPTY_HEADERS,
    EMPTY_PARAMS,
    EMPTY_STRINGS,
    OVERRIDE_KEY,
    BodyCall,
    ErrorClassCall,
    Input,
    Json,
    Object,
    Override,
    OverrideCall,
    RegionCase,
    SignCase,
    SigningCall,
    UrlCase,
    WorkspaceCase,
)


def override_container(spec: dict[str, Json]) -> Override:
    kind, payload = next(iter(spec.items()))
    if kind == "other":
        return payload
    match payload:
        case [*items]:
            listed: list[Json] = items
        case _:
            raise ValueError("override items must be a list")
    if kind == "set":
        return set(listed)
    if kind == "frozenset":
        return frozenset(listed)
    if kind == "tuple":
        return tuple(listed)
    return listed


def resolution_outcome(key: str, resolve: Callable[[], str]) -> Json:
    try:
        value = resolve()
    except AuthenticationError:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L149
        return {"error": "missing_region"}
    except ValueError:
        return {"error": "invalid_region"}
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L106
        return {"error": "other"}
    return {key: value}


def workspaces(cases: list[WorkspaceCase]) -> list[Json]:
    outcomes: list[Json] = []
    for case in cases:
        set_environment(case.get("environment"))
        merged = {
            **(case.get("litellm_settings") or EMPTY_STRINGS),
            **(case.get("deployment") or EMPTY_STRINGS),
        }
        outcomes.append(
            BedrockClaudePlatformMixin._get_workspace_id(  # noqa: SLF001  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L90
                case.get("optional_params") or EMPTY_PARAMS, merged
            )
        )
    return outcomes


def regions(cases: list[RegionCase]) -> list[Json]:
    outcomes: list[Json] = []
    for case in cases:
        set_environment(case.get("environment"))
        resolve_region = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L106
            "Callable[[dict[str, Json]], str]",
            BedrockClaudePlatformMixin()._get_required_aws_region_name,  # noqa: SLF001  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L106
        )
        resolve = partial(resolve_region, case.get("optional_params") or EMPTY_PARAMS)
        outcomes.append(resolution_outcome("region", resolve))
    return outcomes


def urls(cases: list[UrlCase]) -> list[Json]:
    outcomes: list[Json] = []
    for case in cases:
        set_environment(case.get("environment"))
        litellm.api_base = case.get("litellm_api_base")
        resolve_url = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L125
            "Callable[[str | None, str | None, str, dict[str, Json], dict[str, Json]], str]",
            BedrockClaudePlatformMixin().get_complete_url,
        )
        resolve = partial(
            resolve_url,
            case.get("api_base"),
            None,
            "",
            case.get("optional_params") or EMPTY_PARAMS,
            EMPTY_PARAMS,
        )
        outcomes.append(resolution_outcome("url", resolve))
    return outcomes


def override_call(call: OverrideCall) -> Json:
    litellm_params: dict[str, Override] = {}
    deployment = call.get("deployment")
    if deployment is not None:
        litellm_params[OVERRIDE_KEY] = override_container(deployment)
    optional_params: dict[str, Override] = {}
    optional = call.get("optional")
    if optional is not None:
        optional_params[OVERRIDE_KEY] = override_container(optional)
    log_invalid = call.get("log_invalid")
    with captured_warnings() as entries:
        resolved = resolve_unsupported_override(
            litellm_params,
            optional_params=optional_params,
            log_invalid=True if log_invalid is None else log_invalid,
        )
    names: Json = None
    if resolved is not None:
        listed: list[Json] = [name for name in sorted(resolved)]  # noqa: C416  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L51
        names = listed
    warning = entries[0] if entries else None
    return {"names": names, "warning": warning}


def filtered_body_fields(calls: list[BodyCall]) -> list[Json]:
    outcomes: list[Json] = []
    for call in calls:
        unsupported = call.get("unsupported")
        log_dropped = call.get("log_dropped")
        with captured_warnings() as entries:
            body = filter_claude_platform_request_body(
                call.get("params") or EMPTY_PARAMS,
                unsupported_override=None if unsupported is None else frozenset(unsupported),
                log_dropped=True if log_dropped is None else log_dropped,
            )
        entry: Json = {"body": cast(Object, body), "warnings": entries}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L29
        outcomes.append(entry)
    return outcomes


def sign(case: SignCase) -> Json:
    set_environment(case.get("environment"))
    botocore.auth.get_current_datetime = FrozenClock(case["now"])  # type: ignore[misc]  # https://github.com/boto/botocore/blob/444e17a456ee01d28fd0a521936edf2e8da535eb/botocore/auth.py#L433
    BaseAWSLLM.resolve_credentials = CredentialProvider(case["credentials"])  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L512
    signature = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L147
        "tuple[Object, bytes | None]",
        BedrockClaudePlatformMixin().sign_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/claude_platform/common_utils.py#L147
            case.get("headers") or EMPTY_HEADER_MAP,
            case.get("optional_params") or EMPTY_PARAMS,
            case.get("request_data") or EMPTY_PARAMS,
            case["api_base"],
            case.get("api_key"),
            case.get("model"),
        ),
    )
    headers, body = signature
    if body is None:
        return {"signed": False, "headers": headers, "body": None}
    return {
        "signed": True,
        "method": "POST",
        "url": case["api_base"],
        "headers": headers,
        "body": base64.b64encode(body).decode(),
    }


def error_class(case: ErrorClassCall) -> Json:
    error = BedrockClaudePlatformMixin().get_error_class(
        case["message"], case["status_code"], dict(case.get("headers") or EMPTY_HEADERS)
    )
    return {"ok": True, "status_code": error.status_code}


def platform_signing(calls: list[SigningCall]) -> list[Json]:
    outcomes: list[Json] = []
    for call in calls:
        sign_case = call.get("sign")
        if sign_case is not None:
            outcomes.append(sign(sign_case))
            continue
        error_case = call.get("error_class")
        if error_case is not None:
            outcomes.append(error_class(error_case))
    return outcomes


def run(input_value: Input) -> Object:
    if "workspace_alias_precedence" in input_value:
        result: Object = {
            "workspaces": workspaces(input_value["workspace_alias_precedence"]["cases"])
        }
    elif "explicit_region_required" in input_value:
        result = {"regions": regions(input_value["explicit_region_required"]["cases"])}
    elif "api_base_before_region" in input_value:
        result = {"urls": urls(input_value["api_base_before_region"]["cases"])}
    elif "platform_route_prefix" in input_value:
        models: list[Json] = [
            strip_claude_platform_route(model)
            for model in input_value["platform_route_prefix"]["models"]
        ]
        result = {"models": models}
    elif "unsupported_override" in input_value:
        calls = [override_call(call) for call in input_value["unsupported_override"]["calls"]]
        result = {"calls": calls}
    elif "filtered_body_fields" in input_value:
        result = {"calls": filtered_body_fields(input_value["filtered_body_fields"]["calls"])}
    elif "platform_signing" in input_value:
        result = {"calls": platform_signing(input_value["platform_signing"]["calls"])}
    else:
        raise ValueError("the input names exactly one claude_platform_inputs variant")
    return result
