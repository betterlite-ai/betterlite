import logging
from typing import Final

import httpx
from litellm.llms.azure_ai.passthrough.transformation import (
    AzureAIPassthroughConfig,
    foundry_root,
    relay_query_params,
    without_repeated_native_prefix,
)
from pydantic import JsonValue

from .support import (
    EMPTY,
    JSON,
    BaseCall,
    CollectedCall,
    Input,
    NonStreamCall,
    OcrStub,
    PrefixCall,
    QueryCall,
    RootCall,
    ValidationCall,
    WarningRecorder,
    azure_env,
    kind_of,
    litellm_globals,
    make_logging,
    make_response,
)
from .views import OcrView, StandardView, call_type_of

CONFIG: Final = AzureAIPassthroughConfig()


def root_outcome(call: RootCall) -> JsonValue:
    try:
        return {"root": foundry_root(call["api_base"])}
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}


def prefix_outcome(call: PrefixCall) -> JsonValue:
    try:
        return {"root": without_repeated_native_prefix(call["root"], call["native_endpoint"])}
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}


def query_outcome(call: QueryCall) -> JsonValue:
    try:
        params = query_params(call)
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}
    return {"params": params}


def query_params(call: QueryCall) -> JsonValue:
    params = relay_query_params(
        call.get("request_query_params"),
        call.get("deployment_api_version"),
        call["api_base"],
    )
    if params is None:
        return None
    return JSON.validate_python(dict(params))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/passthrough/transformation.py#L75-L85


def base_outcome(call: BaseCall) -> JsonValue:
    env_base = call.get("environment_api_base")
    environment = {} if env_base is None else {"AZURE_AI_API_BASE": env_base}
    params = call.get("litellm_params")
    with azure_env(environment), litellm_globals(call.get("global_api_base"), refresh_enabled=True):
        try:
            url, root = CONFIG.get_complete_url(
                api_base=call.get("api_base"),
                api_key=None,
                model=call["model"],
                endpoint=call["endpoint"],
                request_query_params=call.get("request_query_params"),
                litellm_params={} if params is None else params,
            )
        except (ValueError, httpx.InvalidURL):
            return {"error": {"status": None}}
        return {"url": str(url), "root": root}


def validation_outcome(call: ValidationCall) -> JsonValue:
    environment = call.get("environment")
    if environment is None:
        environment = {}
    with azure_env(environment), litellm_globals(None, refresh_enabled=False):
        try:
            answer = validate_headers(call)
        except (TypeError, ValueError):
            return {"error": {"status": None}}
    return {"headers": JSON.validate_python(answer)}


def validate_headers(call: ValidationCall) -> dict[str, str]:
    params = call.get("params")
    headers = call.get("headers")
    return CONFIG.validate_environment(
        headers={} if headers is None else headers,
        model="",
        messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/passthrough/transformation.py#L122-L137
        optional_params=EMPTY,
        litellm_params={} if params is None else params,
        api_key=call.get("api_key"),
        api_base=call.get("api_base"),
    )


def nonstream_outcome(call: NonStreamCall) -> JsonValue:
    ocr_call = call.get("ocr")
    stub: OcrStub | None = None
    config = CONFIG
    if ocr_call is not None:
        stub = OcrStub(ocr_call)
        config = AzureAIPassthroughConfig(passthrough_ocr=stub)
    logging_obj = make_logging(call["model"], stream=False)
    response = make_response(call)
    recorder = WarningRecorder()
    logger = logging.getLogger("LiteLLM")
    level = logger.level
    logger.setLevel(logging.WARNING)
    logger.addHandler(recorder)
    try:
        answer = config.logging_non_streaming_response(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/passthrough/transformation.py#L139-L166
            model=call["model"],
            custom_llm_provider="azure_ai",
            httpx_response=response,
            request_data=EMPTY,
            logging_obj=logging_obj,
            endpoint=call["endpoint"],
        )
    finally:
        logger.removeHandler(recorder)
        logger.setLevel(level)
    kind = kind_of(type(answer).__name__)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/passthrough/transformation.py#L139-L166
    result: dict[str, JsonValue] = {
        "kind": kind,
        "call_type": call_type_of(logging_obj),
        "ocr_calls": [] if stub is None else stub.calls,
        "warned": bool(recorder.messages),
    }
    if kind == "standard":
        result["standard"] = StandardView.model_validate(answer).response  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/passthrough/transformation.py#L139-L166
    if kind == "ocr":
        view = OcrView.model_validate(answer)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/passthrough/transformation.py#L139-L166
        usage = view.usage_info
        usage_value: JsonValue = None
        if usage is not None:
            usage_value = {
                "pages_processed": usage.pages_processed,
                "credits": usage.credits,
                "doc_size_bytes": usage.doc_size_bytes,
            }
        result["ocr"] = {"model": view.model, "pages": len(view.pages), "usage": usage_value}
    return result


def collected_outcome(call: CollectedCall) -> JsonValue:
    chunks = call.get("chunks")
    if chunks is None:
        chunks = []
    request_data = call.get("request_data")
    if request_data is None:
        request_data = {}
    logging_obj = make_logging(call["model"], stream=True)
    answer = CONFIG.handle_logging_collected_chunks(
        all_chunks=chunks,
        litellm_logging_obj=logging_obj,
        model=call["model"],
        custom_llm_provider="azure_ai",
        endpoint=call["endpoint"],
    )
    kind = kind_of(type(answer).__name__)
    streaming = CONFIG.is_streaming_request(call["endpoint"], request_data)
    return {"kind": kind, "call_type": call_type_of(logging_obj), "streaming": streaming}


def run(input_value: Input) -> JsonValue:
    roots = input_value.get("roots")
    prefixes = input_value.get("prefixes")
    queries = input_value.get("queries")
    bases = input_value.get("bases")
    validations = input_value.get("validations")
    nonstreams = input_value.get("nonstreams")
    streams = input_value.get("streams")
    return {
        "roots": [root_outcome(call) for call in roots] if roots is not None else [],
        "prefixes": [prefix_outcome(call) for call in prefixes] if prefixes is not None else [],
        "queries": [query_outcome(call) for call in queries] if queries is not None else [],
        "bases": [base_outcome(call) for call in bases] if bases is not None else [],
        "validations": (
            [validation_outcome(call) for call in validations] if validations is not None else []
        ),
        "nonstreams": (
            [nonstream_outcome(call) for call in nonstreams] if nonstreams is not None else []
        ),
        "streams": [collected_outcome(call) for call in streams] if streams is not None else [],
    }
