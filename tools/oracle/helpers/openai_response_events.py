import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from functools import partial
from types import ModuleType
from typing import TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.litellm_core_utils.llm_response_utils import convert_dict_to_response
from litellm.llms.openai.common_utils import OpenAIError as LiteLLMOpenAIError
from litellm.llms.openai.responses import transformation
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.llms.openai.workload_identity import OpenAIWorkloadIdentityConfig
from litellm.types.router import GenericLiteLLMParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

litellm.suppress_debug_info = True


class Probe(TypedDict, total=False):
    body: str
    status: int
    headers: dict[str, str]
    now: int
    event: Object
    chunks: list[str]
    model: str | None
    stream: bool | None
    provider: str | None
    native: str
    api_key: str | None
    litellm_api_key: str | None
    litellm_openai_key: str | None
    env_api_key: str | None
    api_base: str | None
    litellm_api_base: str | None
    env_base_url: str | None
    env_api_base: str | None
    identity: bool


class Probes(TypedDict):
    probes: list[Probe]


class FixedTime:
    def __init__(self, seconds: int) -> None:
        self.time = partial(float, seconds)


class Recorder:
    def __init__(self) -> None:
        self.calls: list[Object] = []

    def post_call(self, original_response: str, additional_args: Object) -> None:
        self.calls.append(
            {"original_response": original_response, "additional_args": additional_args}
        )


class NativeAnswer:
    def __init__(self, answer: str) -> None:
        self.answer = answer

    def __call__(self, model: str, custom_llm_provider: str | None) -> bool:  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L739
        match self.answer:
            case "true":
                return True
            case "false":
                return False
            case "other":
                return cast("bool", "sometimes")  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L739
            case _:
                raise ValueError("the fixture catalog lookup failed")


@contextmanager
def environment(values: dict[str, str]) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


@contextmanager
def fixed_time(seconds: int) -> Generator[None]:
    original = convert_dict_to_response.time  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L74
    convert_dict_to_response.time = cast("ModuleType", FixedTime(seconds))  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L88
    try:
        yield
    finally:
        convert_dict_to_response.time = original  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L74


def resolve_fixture(
    api_key: str | None, api_base: str | None
) -> OpenAIWorkloadIdentityConfig | None:
    if api_key is not None:
        return None
    token_file = "default" if api_base is None else api_base
    return OpenAIWorkloadIdentityConfig("idp-fixture", "sa-fixture", token_file)


class TokenFixture:
    def __call__(self, config: OpenAIWorkloadIdentityConfig) -> str:
        return f"wi:{config.token_file}"


@contextmanager
def workload_identity(*, enabled: bool) -> Generator[None]:
    if not enabled:
        yield
        return
    original_resolve = transformation.resolve_openai_workload_identity_config  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604
    original_token = transformation.get_workload_identity_bearer_token  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604
    transformation.resolve_openai_workload_identity_config = resolve_fixture  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604
    transformation.get_workload_identity_bearer_token = TokenFixture()  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604
    try:
        yield
    finally:
        transformation.resolve_openai_workload_identity_config = original_resolve  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604
        transformation.get_workload_identity_bearer_token = original_token  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604


def reply_value(probe: Probe) -> Json:
    body = probe.get("body")
    status = probe.get("status")
    headers = probe.get("headers")
    now = probe.get("now")
    if body is None or status is None or headers is None or now is None:
        raise ValueError("a reply probe needs body, status, headers and now")
    logging_obj = Recorder()
    request = httpx.Request("POST", "https://api.openai.com/v1/responses")
    response = httpx.Response(
        status,
        stream=httpx.ByteStream(body.encode()),
        headers=headers,
        request=request,
    )
    response.read()
    config = OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L558
    with fixed_time(now):
        try:
            reply = config.transform_response_api_response(
                model="gpt-4o",
                raw_response=response,
                logging_obj=cast("LiteLLMLogging", logging_obj),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L563
            )
        except LiteLLMOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L570
            return {"error": error.status_code}
    hidden = cast("Object", reply._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L583
    return {
        "id": reply.id,
        "created_at": reply.created_at,
        "headers": cast("Json", dict(response.headers)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L576
        "additional_headers": hidden["additional_headers"],
        "post_call": logging_obj.calls[0],
    }


def event_value(probe: Probe) -> Json:
    chunk = probe.get("event")
    if chunk is None:
        raise ValueError("an event probe needs event")
    config = OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L558
    event = config.transform_streaming_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L625
        model="gpt-4o",
        parsed_chunk=chunk,
        logging_obj=cast("LiteLLMLogging", None),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L625
    )
    dumped = cast("Object", event.model_dump(exclude_none=True))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L29
    return {"class": type(event).__name__, "event": dumped}


def terminal_chunks(probe: Probe) -> list[str]:
    chunks = probe.get("chunks")
    if chunks is None:
        raise ValueError("a terminal probe needs chunks")
    return chunks


def terminal_response_value(probe: Probe) -> Json:
    response = OpenAIResponsesAPIConfig.parse_terminal_response_from_stream_chunks(
        terminal_chunks(probe)
    )
    if response is None:
        return None
    return {"id": response.id, "created_at": response.created_at}


def terminal_event_value(probe: Probe) -> Json:
    event = OpenAIResponsesAPIConfig.parse_terminal_event_from_stream_chunks(terminal_chunks(probe))
    if event is None:
        return None
    return {
        "class": type(event).__name__,
        "id": event.response.id,
        "created_at": event.response.created_at,
    }


def stream_value(probe: Probe) -> Json:
    native = probe.get("native")
    if native is None:
        raise ValueError("a stream probe needs native")
    original = litellm.utils.supports_native_streaming
    litellm.utils.supports_native_streaming = NativeAnswer(native)
    try:
        return OpenAIResponsesAPIConfig().should_fake_stream(  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L558
            model=probe.get("model"),
            stream=probe.get("stream"),
            custom_llm_provider=probe.get("provider"),
        )
    finally:
        litellm.utils.supports_native_streaming = original


def auth_value(probe: Probe) -> Json:
    given_headers = probe.get("headers")
    headers = {} if given_headers is None else dict(given_headers)
    config = OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L558
    litellm.api_key = probe.get("litellm_api_key")
    litellm.openai_key = probe.get("litellm_openai_key")
    litellm.api_base = None
    given_env = probe.get("env_api_key")
    values: dict[str, str] = {} if given_env is None else {"OPENAI_API_KEY": given_env}
    params = GenericLiteLLMParams(
        api_key=probe.get("api_key"), api_base=probe.get("api_base")
    )
    with environment(values), workload_identity(enabled=probe.get("identity") is True):
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L589
            "Json",
            config.validate_environment(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L589
                headers, model="gpt-4o", litellm_params=params
            ),
        )


def url_value(probe: Probe) -> Json:
    config = OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L558
    litellm.api_base = probe.get("litellm_api_base")
    values: dict[str, str] = {}
    env_base_url = probe.get("env_base_url")
    if env_base_url is not None:
        values["OPENAI_BASE_URL"] = env_base_url
    env_api_base = probe.get("env_api_base")
    if env_api_base is not None:
        values["OPENAI_API_BASE"] = env_api_base
    empty: dict[str, Json] = {}
    with environment(values):
        return config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L604
            api_base=probe.get("api_base"), litellm_params=empty
        )


HANDLERS: dict[str, Callable[[Probe], Json]] = {
    "created_at_conversion": reply_value,
    "lax_response_validation": reply_value,
    "missing_error_code": event_value,
    "unknown_generic_event": event_value,
    "known_event_variants": event_value,
    "completed_incomplete_failed": terminal_response_value,
    "last_terminal_wins": terminal_event_value,
    "no_terminal_event": terminal_event_value,
    "non_native_streaming_model": stream_value,
    "responses_auth_headers": auth_value,
    "responses_base_url": url_value,
}


def run(input_value: dict[str, Probes]) -> Json:
    if len(input_value) != 1:
        raise ValueError("expected one openai_response_events case")
    name = next(iter(input_value))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_response_events case: {name}")
    return [handler(probe) for probe in input_value[name]["probes"]]
