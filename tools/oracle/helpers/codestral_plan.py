import asyncio
import copy
import json
from collections.abc import Awaitable, Iterator
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.codestral.completion.handler import TextCompletionCodestralError
from litellm.types.utils import TextCompletionResponse

# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false, reportArgumentType=false, reportAny=false
if TYPE_CHECKING:
    import functools
# keep exception_type()'s debug banner off stdout; the oracle protocol reads one JSON line
litellm.suppress_debug_info = True
Object: TypeAlias = dict[str, object]
class StopError(Exception):
    pass
class SyncClient:
    def __init__(self) -> None:
        self.calls: list[Object] = []
    def post(
        self, url: str | None = None, headers: object = None,
        data: str = "", stream: object = None,
    ) -> "SyncClient":
        self.calls.append(request_facts(url, headers, data))
        if stream is True:
            return self
        raise StopError
    def iter_lines(self) -> Iterator[str]:
        return iter(())
class Logger:
    def __init__(self) -> None:
        self.events: list[Object] = []
        self.model_call_details: Object = {}
    def pre_call(self, **params: object) -> None:
        self.events.append({"phase": "pre_call", **copy.deepcopy(params)})
    def post_call(self, **params: object) -> None:
        del params
def ignore_verbose(*_args: object) -> None:
    pass
def unreachable(hf_model_name: str) -> Object:
    del hf_model_name
    return {"status": "failure"}
def request_facts(url: str | None, headers: object, data: str) -> Object:
    return {"url": url, "headers": headers, "body": cast(object, json.loads(data))}
def arguments(call: Object, logger: Logger) -> Object:
    return {
        "model": call["model"], "messages": call["messages"],
        "api_base": call.get("api_base"), "custom_prompt_dict": call.get("custom_prompt_dict", {}),
        "model_response": TextCompletionResponse(), "print_verbose": ignore_verbose,
        "encoding": None, "api_key": call["api_key"], "logging_obj": logger,
        "optional_params": call.get("optional_params", {}),
        "timeout": float(cast(float, call["timeout"])),
        "acompletion": call["mode"] == "async", "headers": call.get("headers", {}),
    }
def sync_call(call: Object, logger: Logger) -> object:
    return cast(object, litellm.CodestralTextCompletion().completion(**arguments(call, logger)))
async def async_call(call: Object, logger: Logger) -> object:
    result = litellm.CodestralTextCompletion().completion(**arguments(call, logger))
    return await cast(Awaitable[object], result)
def drive(call: Object, logger: Logger) -> object:
    if call["mode"] == "sync":
        return sync_call(call, logger)
    return asyncio.run(async_call(call, logger))
def facts(call: Object, result: object, sync_client: SyncClient, logger: Logger) -> Object:
    provider: object = None
    request: Object | None = None
    if isinstance(result, CustomStreamWrapper):
        provider = result.custom_llm_provider
        if call["mode"] == "async":
            make_call = cast("functools.partial[object]", result.make_call)
            keywords = cast(Object, make_call.keywords)
            url = cast(str | None, keywords["api_base"])
            request = request_facts(url, keywords["headers"], cast(str, keywords["data"]))
    if request is None and sync_client.calls:
        request = sync_client.calls[0]
    if request is None:
        return {"outcome": "error"}
    return {
        "mode": call["mode"], "request": request, "provider": provider,
        "timeout": None, "events": logger.events,
    }
def run_call(call: Object) -> Object:
    logger = Logger()
    sync_client = SyncClient()
    try:
        with patch.object(litellm, "module_level_client", sync_client):
            result = drive(call, logger)
    except StopError:
        result = None
    except (ValueError, TextCompletionCodestralError):
        return {"outcome": "error"}
    return facts(call, result, sync_client, logger)
def run(data: Object) -> object:
    for name, value in cast(Object, data.get("settings", {})).items():
        if value is not None:
            setattr(litellm.CodestralTextCompletionConfig, name, value)
    with (
        patch.object(huggingface_template_handler, "_get_tokenizer_config", unreachable),
        patch.object(huggingface_template_handler, "_get_chat_template_file", unreachable),
    ):
        return [run_call(call) for call in cast(list[Object], data["calls"])]
