import asyncio
from base64 import b64encode

import httpx
from litellm.llms.bedrock.responses.transformation import (
    BedrockOpenAIResponsesConfig,
    collect_remote_image_urls,
    inline_remote_image_urls,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

from bedrock_mantle_responses.calls import warnings_recorded

from .calls import API_BASE, MODEL, Fixture, FixtureRefusalError, environment, signed_summary
from .models import (
    AuthCase,
    Calls,
    CodexCase,
    FetchCase,
    InputCase,
    OptionCalls,
    OptionCase,
    ReplacementCase,
    RouteCase,
    SigningCase,
    UrlCase,
)

JSON = TypeAdapter(JsonValue)


def route_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[RouteCase]).validate_python(payload):
        config = BedrockOpenAIResponsesConfig.for_model(case["model"])
        provider = None if config is None else config.custom_llm_provider.value
        native = None if config is None else config.supports_native_file_search()
        selected = config is not None
        results.append(
            {"selected": selected, "provider": provider, "native_file_search": native}
        )
    return {"results": results}


def auth_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[AuthCase]).validate_python(payload):
        api_key = case.get("api_key")
        given = case.get("headers")
        headers: dict[str, str] = {} if given is None else {**given}
        with environment(case.get("environment")):
            params = GenericLiteLLMParams(api_key=api_key)
            validated = calls.validate_environment(headers, MODEL, params)
        results.append({"headers": JSON.validate_python(validated)})
    return {"results": results}


def sigv4_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[SigningCase]).validate_python(payload):
        with environment(None):
            headers, body = calls.sign_request(
                case["headers"].copy(), case["optional_params"], case["request_data"], API_BASE
            )
        probe = case.get("error")
        error: JsonValue = None
        if probe is not None:
            probe_headers = httpx.Headers(probe["headers"])
            raised = calls.get_error_class(probe["message"], probe["status"], probe_headers)
            error = {
                "status": raised.status_code,
                "message": raised.message,
                "request_id": raised.response.headers["x-amzn-requestid"],
            }
        encoded = None if body is None else b64encode(body).decode()
        results.append({"signed": signed_summary(headers), "body": encoded, "error": error})
    return {"results": results}


def option_outcome(calls: OptionCalls, case: OptionCase) -> JsonValue:
    with warnings_recorded() as messages:
        drop_params = case.get("drop_params") is True
        mapped = calls.map_openai_params(case["params"], MODEL, drop_params=drop_params)
    native = calls.supports_native_file_search()
    return {"params": mapped, "warnings": messages, "native_file_search": native}


def option_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[OptionCase]).validate_python(payload):
        config = BedrockOpenAIResponsesConfig()
        results.append(option_outcome(config, case))  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/responses/transformation.py#L258-L263
    return {"results": results}


def traversal_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[InputCase]).validate_python(payload)
    results: list[JsonValue] = [[*collect_remote_image_urls(case["input"])] for case in cases]
    return {"results": results}


def replacement_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[ReplacementCase]).validate_python(payload):
        try:
            replaced = inline_remote_image_urls(case["input"], case["inlined"])
        except KeyError:
            results.append({"error": True})
        else:
            results.append({"input": JSON.validate_python(replaced)})
    return {"results": results}


def fetch_outcome(
    calls: Calls, fixture: Fixture, case: FetchCase, *, asynchronous: bool
) -> JsonValue:
    params = GenericLiteLLMParams()
    try:
        if asynchronous:
            body = asyncio.run(
                calls.async_transform_responses_api_request(MODEL, case["input"], {}, params, {})
            )
        else:
            body = calls.transform_responses_api_request(MODEL, case["input"], {}, params, {})
    except FixtureRefusalError:
        return {"error": "fetch", "fetched": [*fixture.fetched]}
    return {"body": body, "fetched": [*fixture.fetched]}


def fetch_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[FetchCase]).validate_python(payload):
        sync_fixture = Fixture(case["data_urls"])
        async_fixture = Fixture(case["data_urls"])
        sync = fetch_outcome(sync_fixture.config(), sync_fixture, case, asynchronous=False)
        parallel = fetch_outcome(async_fixture.config(), async_fixture, case, asynchronous=True)
        results.append({"sync": sync, "async": parallel})
    return {"results": results}


def url_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[UrlCase]).validate_python(payload):
        params = case.get("litellm_params")
        with environment(case.get("environment")):
            try:
                url = calls.get_complete_url(case.get("api_base"), {} if params is None else params)
            except ValueError:
                results.append({"error": True})
            else:
                results.append({"url": url})
    return {"results": results}


def codex_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[CodexCase]).validate_python(payload):
        params = case.get("params")
        given = {} if params is None else params
        with warnings_recorded() as messages:
            body = calls.transform_responses_api_request(
                case["model"], case["input"], given, GenericLiteLLMParams(), {}
            )
        results.append({"body": body, "warnings": messages})
    return {"results": results}
