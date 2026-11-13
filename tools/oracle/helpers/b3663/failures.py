import sys
from collections.abc import Generator, Sequence
from contextlib import contextmanager
from importlib.abc import MetaPathFinder
from importlib.machinery import ModuleSpec
from types import ModuleType, SimpleNamespace
from typing import Literal, TypeAlias, TypedDict, cast

from fastapi.exceptions import HTTPException
from litellm.exceptions import GuardrailRaisedException
from litellm.llms.base_llm.guardrail_translation.utils import unappliable_request_rewrite
from litellm.proxy.guardrails.exception_utils import (
    enrich_http_exception_with_guardrail_context,
    is_fastapi_http_exception,
    pre_call_rejection,
)
from typing_extensions import override

from b3663.usage import Json

Fastapi: TypeAlias = Literal["installed", "absent"]


class IsHttpInput(TypedDict):
    function: Literal["is_http_exception"]
    fastapi: Fastapi
    raised: Literal["http", "guardrail", "other"]
    status_code: int
    block_status_codes: list[int]


class EnrichInput(TypedDict):
    function: Literal["enrich_exception"]
    fastapi: Fastapi
    raised: Literal["http", "other"]
    detail: Json
    guardrail_name: str | None
    event_hook: Json


class PreCallInput(TypedDict):
    function: Literal["pre_call"]
    fastapi: Fastapi
    message: str
    guardrail_name: str | None


class UnappliableInput(TypedDict):
    function: Literal["unappliable"]
    guardrail_name: str | None


Input: TypeAlias = IsHttpInput | EnrichInput | PreCallInput | UnappliableInput


class FastapiBlocker(MetaPathFinder):
    @override
    def find_spec(
        self,
        fullname: str,
        _path: Sequence[str] | None,
        _target: ModuleType | None = None,
    ) -> ModuleSpec | None:
        if fullname == "fastapi.exceptions":
            raise ModuleNotFoundError(fullname)
        return None


@contextmanager
def fastapi_state(fastapi: Fastapi) -> Generator[None]:
    if fastapi == "installed":
        yield
        return
    saved = sys.modules.pop("fastapi.exceptions")
    blocker = FastapiBlocker()
    sys.meta_path.insert(0, blocker)
    try:
        yield
    finally:
        sys.meta_path.remove(blocker)
        sys.modules["fastapi.exceptions"] = saved


def raised_error(
    raised: Literal["http", "guardrail", "other"], status_code: int, detail: Json
) -> Exception:
    match raised:
        case "http":
            return HTTPException(status_code=status_code, detail=detail)
        case "guardrail":
            return GuardrailRaisedException(
                guardrail_name="guard", message="blocked", status_code=status_code
            )
        case "other":
            return ValueError("other")


def pre_call(input_value: PreCallInput) -> Json:
    with fastapi_state(input_value["fastapi"]):
        rejection = pre_call_rejection(input_value["message"], input_value["guardrail_name"])
    try:
        raise rejection
    except HTTPException as http:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/exception_utils.py#L39
        detail = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/exception_utils.py#L39
            Json, http.detail
        )
        return {
            "kind": "http",
            "status_code": http.status_code,
            "detail": detail,
        }
    except GuardrailRaisedException as raised:
        return {
            "kind": "guardrail",
            "guardrail_name": raised.guardrail_name,
            "message": raised.message,
            "status_code": raised.status_code,
            "blocked_content": raised.blocked_content,
        }


def run(input_value: Input) -> Json:
    match input_value["function"]:
        case "is_http_exception":
            error = raised_error(input_value["raised"], input_value["status_code"], None)
            with fastapi_state(input_value["fastapi"]):
                found = is_fastapi_http_exception(error, input_value["block_status_codes"])
            return {"is_http": found}
        case "enrich_exception":
            detail = input_value["detail"]
            callback = SimpleNamespace(
                guardrail_name=input_value["guardrail_name"], event_hook=input_value["event_hook"]
            )
            with fastapi_state(input_value["fastapi"]):
                enrich_http_exception_with_guardrail_context(
                    raised_error(input_value["raised"], 400, detail), callback
                )
            return {"detail": detail}
        case "pre_call":
            return pre_call(input_value)
        case "unappliable":
            error = unappliable_request_rewrite(input_value["guardrail_name"])
            return {"guardrail_name": error.guardrail_name, "message": str(error)}
