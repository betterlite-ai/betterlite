import logging
from collections.abc import Callable
from decimal import Decimal
from logging.handlers import BufferingHandler
from typing import Generic, Literal, NotRequired, TypeAlias, TypedDict, TypeVar, cast

from litellm._logging import verbose_logger
from litellm.litellm_core_utils import core_helpers as core
from litellm.litellm_core_utils import llm_request_utils as llm_request
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import ModelResponseStream
from pydantic import TypeAdapter
from starlette.exceptions import HTTPException

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
STATUS = TypeAdapter(int)
LOGGER = logging.getLogger("oracle.call_helpers")
Name = TypeVar("Name")
Item = TypeVar("Item")


class Values(TypedDict, Generic[Name, Item]):
    function: Name
    values: list[Item]


class Steps(TypedDict, Generic[Name, Item]):
    function: Name
    steps: list[Item]


class Fact(TypedDict, total=False):
    code: Json
    status_code: Json
    llm_provider: str | None
    category: str | None
    http_exception: bool
    base_llm_exception: bool


class Division(TypedDict):
    numerator: float
    denominator: float
    default: float


class SecondsDivision(TypedDict):
    seconds: float
    denominator: float
    default: NotRequired[float | None]


class ModelName(TypedDict):
    model: str
    provider: NotRequired[str | None]
    deployment: NotRequired[str | None]


class StampedToken(TypedDict):
    function: Literal["litellm_metadata", "stamped_oauth_token"]
    metadata: NotRequired[Object | None]
    litellm_params: NotRequired[Object | None]


class SpendMetadata(TypedDict):
    function: Literal["add_missing_spend_metadata"]
    litellm_metadata: Object
    metadata: Object


class Project(TypedDict):
    function: Literal["project", "set_hidden"]
    headers: dict[str, str]
    existing: NotRequired[Object]


class GetHidden(TypedDict):
    function: Literal["get_hidden"]
    hidden: NotRequired[Json]


class StreamAttributes(TypedDict):
    function: Literal["stream_attributes"]
    response: Object
    original_chunk: Object


class UpstreamHeader(TypedDict):
    function: Literal["upstream_internal_header"]
    upstream: dict[str, str]
    trusted: Object


class Snapshot(TypedDict):
    function: Literal["snapshot_isolation"]
    record: dict[str, Object]


class Filter(TypedDict):
    function: Literal["filter_internal"]
    record: Object
    additional: list[str]


Trusted: TypeAlias = StampedToken | SpendMetadata
Header: TypeAlias = Project | GetHidden | StreamAttributes
Listed: TypeAlias = (
    Steps[Literal["metadata_bucket"], dict[str, Object]]
    | Steps[Literal["trusted_flags"], Trusted]
    | Steps[Literal["response_headers"], Header]
    | Steps[Literal["request_headers"], Object | None]
    | Values[Literal["model_reconstruction"], ModelName]
)
Rest: TypeAlias = UpstreamHeader | Snapshot | Filter | Listed
Input: TypeAlias = (
    Values[Literal["finish_reasons"], str]
    | Values[Literal["token_limits"], Json]
    | Values[Literal["drop_request_invalid"], Json]
    | Values[Literal["drop_env_invalid"], str | None]
    | Values[Literal["safe_divide"], Division]
    | Values[Literal["safe_divide_seconds"], SecondsDivision]
    | Values[Literal["expected_client_error"], Fact | None]
    | Rest
)


class _PlainError(Exception):
    code: Json
    status_code: Json
    llm_provider: str | None
    category: str | None


class _HttpError(HTTPException):
    code: Json
    llm_provider: str | None
    category: str | None


class _ProviderError(BaseLLMException):
    code: Json
    llm_provider: str | None
    category: str | None


class _Hidden:
    def __init__(self, hidden: Json) -> None:
        self._hidden_params = hidden


def _expected(fact: Fact) -> bool:
    error: _PlainError | _HttpError | _ProviderError
    if fact.get("http_exception"):
        error = _HttpError(STATUS.validate_python(fact.get("status_code"), strict=True))
    elif fact.get("base_llm_exception"):
        status = STATUS.validate_python(fact.get("status_code"), strict=True)
        error = _ProviderError(status, "fixture")
    else:
        error = _PlainError()
        error.status_code = fact.get("status_code")
    error.code = fact.get("code")
    error.llm_provider = fact.get("llm_provider")
    error.category = fact.get("category")
    return core.is_expected_client_error(error)


def _finish_reason(raw: str) -> Json:
    capture = BufferingHandler(capacity=100)
    verbose_logger.addHandler(capture)
    reason = core.map_finish_reason(raw)
    verbose_logger.removeHandler(capture)
    return {"reason": reason, "recognized": len(capture.buffer) == 0}


def _drop_flag(value: Json, environ: dict[str, str] | None) -> Json:
    capture = BufferingHandler(capacity=100)
    LOGGER.addHandler(capture)
    result: Object = {}
    if environ is None:
        result["normalized"] = core.normalize_drop_params(value)
        result["enabled"] = core.drop_params_flag(value, "call_helpers", LOGGER)
    else:
        result["enabled"] = core.drop_params_env_flag(environ, LOGGER)
    LOGGER.removeHandler(capture)
    result["warned"] = len(capture.buffer) > 0
    return result


def _trusted(step: Trusted) -> Json:
    if step["function"] == "add_missing_spend_metadata":
        internal = dict(step["litellm_metadata"])
        return core.add_missing_spend_metadata_to_litellm_metadata(internal, step["metadata"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L282
    if step["function"] == "litellm_metadata":
        params = step.get("litellm_params")
        kwargs: Object = {} if params is None else {"litellm_params": params}
        return core.get_litellm_metadata_from_kwargs(kwargs)  # type: ignore[misc, no-any-return]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L349
    metadata = step.get("metadata")
    stamped = core.proxy_stamped_used_client_oauth_token(metadata, step.get("litellm_params"))
    return cast(Json, stamped)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L342


def _header(step: Header) -> Json:
    if step["function"] == "project":
        return core.process_response_headers(step["headers"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L453
    if step["function"] == "set_hidden":
        hidden: Object = {"additional_headers": step.get("existing")}
        core.set_provider_response_headers_in_hidden_params(_Hidden(hidden), step["headers"])  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L758
        return {"headers": hidden["headers"], "additional_headers": hidden["additional_headers"]}
    if step["function"] == "get_hidden":
        headers = core.get_provider_response_headers_from_hidden_params(_Hidden(step.get("hidden")))
        return None if headers is None else dict(headers)
    response = ModelResponseStream.model_validate(step["response"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    original = ModelResponseStream.model_validate(step["original_chunk"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    before: Object = response.model_dump()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2096
    core.preserve_upstream_non_openai_attributes(response, original)
    after: Object = response.model_dump()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2096
    return {key: value for key, value in after.items() if key not in before or before[key] != value}


def _bucket(slots: dict[str, Object]) -> Json:
    request: Object = dict(slots)
    name, bucket = core.get_or_create_metadata_bucket(request)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L323
    return {"name": name, "bucket": bucket}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L325


def _model_name(entry: ModelName) -> Json:
    deployment = entry.get("deployment")
    metadata: Object = {} if deployment is None else {"deployment": deployment}
    rebuild = cast(Callable[[str, str | None, Object], str], core.reconstruct_model_name)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L409
    return rebuild(entry["model"], entry.get("provider"), metadata)


def _listed(input_value: Listed) -> list[Json]:
    if input_value["function"] == "metadata_bucket":
        return [_bucket(slots) for slots in input_value["steps"]]
    if input_value["function"] == "trusted_flags":
        return [_trusted(step) for step in input_value["steps"]]
    if input_value["function"] == "response_headers":
        return [_header(step) for step in input_value["steps"]]
    if input_value["function"] == "model_reconstruction":
        return [_model_name(entry) for entry in input_value["values"]]
    return [llm_request.get_proxy_server_request_headers(params) for params in input_value["steps"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_request_utils.py#L170


def _rest(input_value: Rest) -> Json:
    if input_value["function"] == "upstream_internal_header":
        upstream = core.process_response_headers(input_value["upstream"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L453
        trusted = core.process_response_headers(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L455
            input_value["trusted"], preserve_litellm_internal_headers=True
        )
        return {"upstream": upstream, "trusted": trusted}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L456
    if input_value["function"] == "filter_internal":
        names = set(input_value["additional"])
        return core.filter_internal_params(input_value["record"], names)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L683
    if input_value["function"] == "snapshot_isolation":
        record = input_value["record"]
        snapshot = cast(dict[str, Object], core.safe_deep_copy(record))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L514
        for value in snapshot.values():
            value["edited"] = True
        source: Object = dict(record)
        copied: Object = dict(snapshot)
        return {"source": source, "snapshot": copied}
    return _listed(input_value)


def _seconds(pair: SecondsDivision) -> Json:
    quotient = core.safe_divide_seconds(**pair)
    return None if quotient is None else format(Decimal(str(quotient)).normalize(), "f")


def run(input_value: Input) -> Json:
    result: Json
    if input_value["function"] == "finish_reasons":
        result = [_finish_reason(raw) for raw in input_value["values"]]
    elif input_value["function"] == "token_limits":
        result = [core.coerce_token_limit(value) for value in input_value["values"]]
    elif input_value["function"] == "drop_request_invalid":
        result = [_drop_flag(value, None) for value in input_value["values"]]
    elif input_value["function"] == "drop_env_invalid":
        result = [
            _drop_flag(None, {} if value is None else {"LITELLM_DROP_PARAMS": value})
            for value in input_value["values"]
        ]
    elif input_value["function"] == "expected_client_error":
        result = [
            core.is_expected_client_error(None) if fact is None else _expected(fact)
            for fact in input_value["values"]
        ]
    elif input_value["function"] == "safe_divide":
        result = [
            format(Decimal(str(core.safe_divide(**pair))).normalize(), "f")
            for pair in input_value["values"]
        ]
    elif input_value["function"] == "safe_divide_seconds":
        result = [_seconds(pair) for pair in input_value["values"]]
    else:
        result = _rest(input_value)
    return result
