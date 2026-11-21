import subprocess
import sys
from typing import Final

import pytest
from pydantic import JsonValue, TypeAdapter

UNPACK_METHODS: Final = frozenset({
    "async_build_agentic_loop_plan", "async_build_chat_completion_agentic_loop_plan",
    "async_get_chat_completion_prompt", "async_log_event", "async_pre_routing_hook",
    "async_run_agentic_loop", "async_run_chat_completion_agentic_loop",
    "async_should_run_agentic_loop", "async_should_run_chat_completion_agentic_loop",
    "get_chat_completion_prompt", "log_event",
})
NONE_HOOKS: Final = frozenset({
    "async_agentic_run", "async_chat_run", "async_cleanup", "async_log_failure_event",
    "async_log_pre_api_call", "async_log_stream_event", "async_log_success_event", "audit_log",
    "callback_failure", "cold_storage", "failure_deployment", "failure_event",
    "failure_fallback", "headers", "mcp_tool_call", "moderation",
    "post_api_call", "post_call_failure", "post_call_success", "pre_api_call",
    "pre_call", "pre_call_check", "pre_call_check_sync", "pre_call_deployment",
    "pre_request", "pre_routing", "rate_limit", "stream_event",
    "streaming_deployment", "streaming_hook", "success_deployment", "success_event",
    "success_fallback", "translate_input", "translate_output", "translate_streaming",
})
CUSTOM_LOGGER_ALL: Final = ["CustomLogger"]
CUSTOM_BATCH_ALL: Final = ["CustomBatchLogger"]
CUSTOM_LOGGER_TYPES_ALL: Final = [
    "CHAT_COMPLETION_AGENTIC_SURFACE", "CODE_INTERPRETER_INTERCEPTION_PREFIX",
    "CONVERTED_STREAM_KEYS", "HEADROOM_CONVERTED_STREAM_KEY",
    "HEADROOM_INTERCEPTION_PREFIX", "INTERCEPTION_INTERNAL_PREFIXES",
    "NON_CODE_INTERPRETER_INTERCEPTION_INTERNAL_PREFIXES", "RESPONSES_AGENTIC_SURFACE",
    "AgenticLoopPlan", "AgenticLoopRequestPatch", "AgenticLoopSafetyError",
    "StandardCustomLoggerInitParams", "converted_stream_requested",
    "is_interception_internal_key",
]
BATCH_SIZE_DEFAULT: Final = 512
FLUSH_INTERVAL_DEFAULT: Final = 5
MAX_QUEUE_DEFAULT: Final = 50_000
LATE_BATCH_SIZE: Final = 7
LATE_FLUSH_INTERVAL: Final = 3
CUSTOM_BATCH_SIZE: Final = 2
CUSTOM_FLUSH_INTERVAL: Final = 1.5
CUSTOM_MAX_QUEUE: Final = 7
SMALL_NUMBER: Final = 5
ZERO: Final = 0
ONE: Final = 1
TWO: Final = 2
EVENT_ARG_COUNT: Final = 4
MAX_STR_LENGTH: Final = 10_000
BASE64_MARKER: Final = "[BASE64_REDACTED]"
MAX_DEPTH_MARKER: Final = "[MAX_DEPTH_REACHED]"
REDACTED_CONTENT: Final = "redacted-by-litellm"
TRUNCATION_SUFFIX: Final = "...truncated by litellm, this logger does not support large content"
PROBE: Final = """
import asyncio
import importlib
import inspect
import json
import os
import sys
from typing import get_args

os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

import betterlite
import litellm

PACKAGES = {"litellm": litellm, "betterlite": betterlite}
DATA = "data:image/png;base64,iVBORw0KGgo="
LIMIT = 10_000
LONG = 10_001
LOGGER = "integrations.custom_logger"
BATCH = "integrations.custom_batch_logger"
TYPES = "types.integrations.custom_logger"

def logger_class(package):
    return importlib.import_module(package.__name__ + "." + LOGGER).CustomLogger

def batch_class(package):
    return importlib.import_module(package.__name__ + "." + BATCH).CustomBatchLogger

def unpack_keys(member):
    parameter = inspect.signature(member).parameters.get("params")
    if parameter is None:
        return []
    arguments = get_args(parameter.annotation)
    return sorted(getattr(arguments[0] if arguments else None, "__annotations__", {}))

def param_row(item):
    default = "required"
    if item.default is not inspect.Parameter.empty:
        default = repr(item.default)
    return {"name": item.name, "kind": item.kind.name, "default": default}

def method_rows(cls):
    rows = {}
    for name, member in inspect.getmembers(cls, inspect.isfunction):
        if name.startswith("__") and name != "__init__":
            continue
        rows[name] = {
            "is_async": inspect.iscoroutinefunction(member) or inspect.isasyncgenfunction(member),
            "params": [param_row(i) for i in inspect.signature(member).parameters.values()],
            "unpack_keys": unpack_keys(member),
        }
    return rows

def module_view(package, suffix):
    module = importlib.import_module(package.__name__ + "." + suffix)
    exported = list(getattr(module, "__all__", []))
    other = importlib.import_module("litellm." + suffix)
    missing = [name for name in exported if not hasattr(other, name)]
    if package.__name__ != "betterlite":
        return {"all": exported, "missing_in_litellm": missing, "same_objects": True}
    registry = importlib.import_module("betterlite._integrations")
    same = all(getattr(module, name) is getattr(registry, name) for name in exported)
    return {"all": exported, "missing_in_litellm": missing, "same_objects": same}

def subclass_probe(base):
    class Recorder(base):
        def __init__(self):
            super().__init__()
            self.seen = []
        def log_pre_api_call(self, model, messages, kwargs):
            self.seen.append(model)
        async def async_filter_listed_models(self, user_api_key_dict, model_names):
            return [name for name in model_names if "keep" in name]
    recorder = Recorder()
    recorder.log_pre_api_call("model-one", [], {})
    filtered = asyncio.run(recorder.async_filter_listed_models(None, ["keep-me", "drop-me"]))
    return {"seen": recorder.seen, "filtered": list(filtered),
            "message_logging": recorder.message_logging,
            "turn_off_message_logging": recorder.turn_off_message_logging}

def type_report(package):
    module = importlib.import_module(package.__name__ + "." + TYPES)
    keys = ["model", "_code_interpreter_interception_x", "_headroom_interception_x",
            "_headroom_interception_converted_stream", "_websearch_interception",
            "_compression_interception", "_unknown"]
    params = [{"_headroom_interception_converted_stream": True},
              {"_headroom_interception_converted_stream": 1},
              {"_headroom_interception_converted_stream": 0},
              {"_code_interpreter_interception_converted_stream": "yes"},
              {"model": "x"}, {}]
    init_cls = module.StandardCustomLoggerInitParams; patch_cls = module.AgenticLoopRequestPatch
    safety = module.AgenticLoopSafetyError
    return {
        "is_internal": [module.is_interception_internal_key(key) for key in keys],
        "converted": [module.converted_stream_requested(item) for item in params],
        "prefixes": sorted(module.INTERCEPTION_INTERNAL_PREFIXES),
        "converted_keys": sorted(module.CONVERTED_STREAM_KEYS),
        "init_params": [init_cls().model_dump(),
                        init_cls(turn_off_message_logging=True).model_dump(),
                        init_cls(turn_off_message_logging=None).model_dump()],
        "patch": patch_cls().model_dump(),
        "patch_set": patch_cls(
            model="m", messages=[{"a": 1}], optional_params={"t": 1}
        ).model_dump(),
        "safety_error": [safety.__name__, issubclass(safety, ValueError)],
    }

def surface_report(package):
    logger_cls = logger_class(package); batch_cls = batch_class(package)
    suffixes = (LOGGER, BATCH, TYPES)
    return {
        "custom_logger_methods": method_rows(logger_cls),
        "custom_batch_methods": method_rows(batch_cls),
        "class_vars": {
            "server_fulfilled_tool_names": sorted(logger_cls.server_fulfilled_tool_names),
            "enforces_request_content": logger_cls.enforces_request_content,
            "preserve_events_added_during_flush": batch_cls.preserve_events_added_during_flush,
            "default_max_queue_size": batch_cls.DEFAULT_MAX_QUEUE_SIZE,
        },
        "modules": {suffix: module_view(package, suffix) for suffix in suffixes},
        "subclass": subclass_probe(logger_cls),
        "types": type_report(package),
    }

def init_report(cls):
    default = cls()
    flagged = cls(turn_off_message_logging=True)
    unflagged = cls(message_logging=False)
    both = cls(turn_off_message_logging=True, message_logging=False)
    try:
        cls(True)
        positional = "none"
    except TypeError:
        positional = "TypeError"
    return {"default_message_logging": default.message_logging,
            "default_turn_off": default.turn_off_message_logging,
            "flagged_message_logging": flagged.message_logging,
            "flagged_turn_off": flagged.turn_off_message_logging,
            "unflagged_message_logging": unflagged.message_logging,
            "unflagged_turn_off": unflagged.turn_off_message_logging,
            "both_message_logging": both.message_logging,
            "both_turn_off": both.turn_off_message_logging,
            "positional_error": positional}

def events_report(cls):
    logger = cls()
    result = {}
    calls, seen, verbose = [], {}, []
    def record(*args):
        calls.append(len(args))
        seen["kwargs"] = args[0]
    def boom(*args):
        raise RuntimeError("boom")
    kwargs = {"model": "old-model"}
    logger.log_input_event("new-model", [], kwargs, verbose.append, record)
    result["input_calls"] = calls[0]
    result["input_same_kwargs"] = seen["kwargs"] is kwargs
    result["input_model"] = kwargs["model"]
    result["input_type"] = kwargs["log_event_type"]
    result["input_verbose"] = verbose[0]
    try:
        logger.log_input_event("m", [], {}, verbose.append, boom)
        result["input_error"] = "swallowed"
    except RuntimeError:
        result["input_error"] = "raised"
    result["input_verbose_count"] = len(verbose)
    result["input_error_message"] = verbose[1] if len(verbose) > 1 else None
    event_calls, event_verbose = [], []
    def record_event(*args):
        event_calls.append(len(args))
    event_kwargs = {}
    logger.log_event(kwargs=event_kwargs, response_obj={"id": "r"}, start_time=1.0,
                     end_time=2.0, print_verbose=event_verbose.append,
                     callback_func=record_event)
    result["event_calls"] = event_calls[0]
    result["event_type"] = event_kwargs["log_event_type"]
    result["event_verbose_count"] = len(event_verbose)
    try:
        logger.log_event(kwargs={}, response_obj={"id": "r"}, start_time=1.0, end_time=2.0,
                         print_verbose=event_verbose.append, callback_func=boom)
        result["event_error"] = "swallowed"
    except RuntimeError:
        result["event_error"] = "raised"
    result["event_error_verbose"] = len(event_verbose)
    result["event_error_message"] = event_verbose[0] if event_verbose else None
    async def async_events():
        a_result = {}
        a_calls, a_seen, a_verbose = [], {}, []
        async def a_record(*args):
            a_calls.append(len(args))
            a_seen["kwargs"] = args[0]
        async def a_boom(*args):
            raise RuntimeError("boom")
        a_kwargs = {"model": "old-model"}
        await logger.async_log_input_event("new-model", [], a_kwargs, a_verbose.append, a_record)
        a_result["async_input_calls"] = a_calls[0]
        a_result["async_input_same_kwargs"] = a_seen["kwargs"] is a_kwargs
        a_result["async_input_type"] = a_kwargs["log_event_type"]
        a_result["async_input_verbose"] = a_verbose[0]
        try:
            await logger.async_log_input_event("m", [], {}, a_verbose.append, a_boom)
            a_result["async_input_error"] = "swallowed"
        except RuntimeError:
            a_result["async_input_error"] = "raised"
        a_result["async_input_error_message"] = a_verbose[1] if len(a_verbose) > 1 else None
        e_calls, e_verbose = [], []
        async def e_record(*args):
            e_calls.append(len(args))
        e_kwargs = {}
        await logger.async_log_event(kwargs=e_kwargs, response_obj={"id": "r"}, start_time=1.0,
                                     end_time=2.0, print_verbose=e_verbose.append,
                                     callback_func=e_record)
        a_result["async_event_calls"] = e_calls[0]
        a_result["async_event_type"] = e_kwargs["log_event_type"]
        a_result["async_event_verbose_count"] = len(e_verbose)
        try:
            await logger.async_log_event(kwargs={}, response_obj={"id": "r"}, start_time=1.0,
                                         end_time=2.0, print_verbose=e_verbose.append,
                                         callback_func=a_boom)
            a_result["async_event_error"] = "swallowed"
        except RuntimeError:
            a_result["async_event_error"] = "raised"
        a_result["async_event_error_verbose"] = len(e_verbose)
        a_result["async_event_error_message"] = e_verbose[0] if e_verbose else None
        return a_result
    result.update(asyncio.run(async_events()))
    return result

def hooks_report(package):
    cls = logger_class(package)
    logger = cls()
    nv = {}
    nv["pre_api_call"] = logger.log_pre_api_call("m", [], {})
    nv["post_api_call"] = logger.log_post_api_call({}, {}, 1.0, 2.0)
    nv["stream_event"] = logger.log_stream_event({}, {}, 1.0, 2.0)
    nv["success_event"] = logger.log_success_event({}, {}, 1.0, 2.0)
    nv["failure_event"] = logger.log_failure_event({}, {}, 1.0, 2.0)
    nv["pre_call_check_sync"] = logger.pre_call_check({})
    nv["callback_failure"] = logger.handle_callback_failure("x")
    nv["translate_input"] = logger.translate_completion_input_params({})
    nv["translate_output"] = logger.translate_completion_output_params(None)
    nv["translate_streaming"] = logger.translate_completion_output_params_streaming(None)
    async def run_all():
        v = {}
        deployments = [{"model": "m1"}]
        response = {"id": "r1"}
        plan_kwargs = {"tools": {}, "model": "m", "messages": [], "response": None,
                       "anthropic_messages_provider_config": None,
                       "anthropic_messages_optional_request_params": {},
                       "logging_obj": None, "stream": False, "kwargs": {}}
        chat_kwargs = {"tools": {}, "model": "m", "messages": [], "response": None,
                       "optional_params": {}, "logging_obj": None, "stream": False, "kwargs": {}}
        gate_kwargs = {"response": None, "model": "m", "messages": [], "tools": None,
                       "stream": False, "custom_llm_provider": "openai", "kwargs": {}}
        nv["async_log_stream_event"] = await logger.async_log_stream_event({}, {}, 1.0, 2.0)
        nv["async_log_pre_api_call"] = await logger.async_log_pre_api_call("m", [], {})
        nv["async_log_success_event"] = await logger.async_log_success_event({}, {}, 1.0, 2.0)
        nv["async_log_failure_event"] = await logger.async_log_failure_event({}, {}, 1.0, 2.0)
        nv["audit_log"] = await logger.async_log_audit_log_event({})
        nv["pre_request"] = await logger.async_pre_request_hook("m", [], {})
        nv["pre_routing"] = await logger.async_pre_routing_hook(model="m", request_kwargs={})
        nv["pre_call"] = await logger.async_pre_call_hook(None, None, {}, "completion")
        nv["pre_call_deployment"] = await logger.async_pre_call_deployment_hook({}, None)
        nv["pre_call_check"] = await logger.async_pre_call_check({}, None)
        nv["success_deployment"] = await logger.async_post_call_success_deployment_hook(
            {}, {"id": 1}, None)
        nv["failure_deployment"] = await logger.async_post_call_failure_deployment_hook(
            {}, ValueError("x"), None)
        nv["streaming_deployment"] = await logger.async_post_call_streaming_deployment_hook(
            {}, {"c": 1}, None)
        nv["rate_limit"] = await logger.log_model_group_rate_limit_error(ValueError("x"), None, {})
        nv["success_fallback"] = await logger.log_success_fallback_event("m", {}, ValueError("x"))
        nv["failure_fallback"] = await logger.log_failure_fallback_event("m", {}, ValueError("x"))
        nv["headers"] = await logger.async_post_call_response_headers_hook({}, None, None)
        nv["post_call_failure"] = await logger.async_post_call_failure_hook(
            {}, ValueError("x"), None)
        nv["post_call_success"] = await logger.async_post_call_success_hook({}, None, None)
        nv["moderation"] = await logger.async_moderation_hook({}, None, "completion")
        nv["streaming_hook"] = await logger.async_post_call_streaming_hook(None, "chunk")
        nv["mcp_tool_call"] = await logger.async_post_mcp_tool_call_hook({}, None, None, None)
        nv["cold_storage"] = (
            await logger.get_proxy_server_request_from_cold_storage_with_object_key("k"))
        nv["async_agentic_run"] = await logger.async_run_agentic_loop(**plan_kwargs)
        nv["async_chat_run"] = await logger.async_run_chat_completion_agentic_loop(**chat_kwargs)
        plan = await logger.async_build_agentic_loop_plan(**plan_kwargs)
        v["agentic_plan"] = json.loads(plan.model_dump_json())
        chat_plan = await logger.async_build_chat_completion_agentic_loop_plan(**chat_kwargs)
        v["chat_plan"] = json.loads(chat_plan.model_dump_json())
        nv["async_cleanup"] = await logger.async_agentic_loop_cleanup_hook(plan, {})
        v["agentic_gate"] = list(await logger.async_should_run_agentic_loop(**gate_kwargs))
        v["chat_gate"] = list(
            await logger.async_should_run_chat_completion_agentic_loop(**gate_kwargs))
        v["agentic_post_same"] = (
            await logger.async_post_agentic_loop_response_hook(response, plan, {})
        ) is response
        v["filter_deployments_same"] = (
            await logger.async_filter_deployments("m", deployments, None)) is deployments
        names = ["keep-a", "drop-b"]
        v["filter_models_same"] = (await logger.async_filter_listed_models(None, names)) is names
        v["filter_models"] = list(names)
        prompt_kwargs = {"model": "m", "messages": [{"role": "user", "content": "hi"}],
                         "non_default_params": {"temperature": 0.5}, "prompt_id": None,
                         "prompt_variables": None, "dynamic_callback_params": {}}
        v["prompt"] = list(logger.get_chat_completion_prompt(**prompt_kwargs))
        async_kwargs = dict(prompt_kwargs)
        async_kwargs["litellm_logging_obj"] = None
        v["async_prompt"] = list(await logger.async_get_chat_completion_prompt(**async_kwargs))
        hook_kwargs = {"a": 1}
        hook_response = {"r": 1}
        returned = logger.logging_hook(hook_kwargs, hook_response, "completion")
        v["logging_hook_same"] = returned[0] is hook_kwargs and returned[1] is hook_response
        async_returned = await logger.async_logging_hook(hook_kwargs, hook_response, "completion")
        v["async_logging_hook_same"] = (
            async_returned[0] is hook_kwargs and async_returned[1] is hook_response
        )
        async def chunks():
            yield 1
            yield 2
        streamed = logger.async_post_call_streaming_iterator_hook(None, chunks(), {})
        v["streaming_iterator"] = [item async for item in streamed]
        try:
            await logger.async_dataset_hook(None, None)
            v["dataset_error_type"] = "none"
            v["dataset_error_text"] = ""
        except NotImplementedError as error:
            v["dataset_error_type"] = type(error).__name__
            v["dataset_error_text"] = str(error)
        v["env_vars"] = [cls.get_callback_env_vars(None),
                         cls.get_callback_env_vars("not-a-callback")]
        v["redacts_itself"] = logger.redacts_messages_itself()
        return v
    v = asyncio.run(run_all())
    flags = {key: value is None for key, value in nv.items()}
    return {"none": flags, "values": v, "events": events_report(cls), "init": init_report(cls)}

def response_view(response):
    return {"has_choices": isinstance(response, dict) and "choices" in response,
            "content": response["choices"][0]["message"]["content"]}

def helpers_report(package):
    cls = logger_class(package)
    logger = cls(); redacting = cls(turn_off_message_logging=True)
    s = {}
    s["truncate_text_long"] = logger._truncate_text("x" * LONG, LIMIT)
    s["truncate_text_short"] = logger._truncate_text("short", 10)
    s["truncate_field_none"] = logger._truncate_field(None, 10)
    s["truncate_field_short"] = logger._truncate_field("abc", 10)
    s["truncate_field_list"] = logger._truncate_field(["z" * 20], 10)
    exact = "x" * LIMIT
    s["truncate_exact"] = [logger._truncate_text(exact, LIMIT),
                           logger._truncate_field(exact, LIMIT)]
    payload = {"id": "x", "error_str": "E" * LONG,
               "messages": [{"role": "user", "content": "M" * LONG}],
               "response": "short", "extra": {"keep": True}}
    snapshot = json.loads(json.dumps(payload))
    s["truncated"] = logger.truncate_standard_logging_payload_content(payload)
    s["truncated_input_unchanged"] = payload == snapshot
    s["metadata_fields"] = [logger._select_metadata_field(),
                            logger._select_metadata_field({}),
                            logger._select_metadata_field({"litellm_metadata": {"a": 1}}),
                            logger._select_metadata_field(
                                {"metadata": {}, "litellm_metadata": {}})]
    contents = [{"type": "text"}, {"type": "file", "x": 1}, {"type": "image_url"}, {},
                {"type": None}, {"type": ""}, "string", None, 5]
    s["keep_content"] = [logger._should_keep_content(content) for content in contents]
    values = [DATA, {"url": DATA}, [DATA, {"x": "data:audio/wav;base64,AAAA"}], "plain", 5,
              None, True]
    s["redact_base64"] = [logger._redact_base64(value) for value in values]
    s["redact_depth"] = logger._redact_base64([[[[DATA]]]], 0, 2)
    messages = [
        {"role": "user", "content": [
            {"type": "text", "text": "see " + DATA},
            {"type": "image_url", "image_url": {"url": DATA}},
            {"type": "file", "file": {"file_data": DATA}}]},
        {"role": "assistant", "content": "plain " + DATA},
        {"role": "system", "content": 0},
    ]
    s["processed"] = logger._process_messages(json.loads(json.dumps(messages)))
    strip_payload = {"messages": json.loads(json.dumps(messages)), "model": "m"}
    s["stripped_sync"] = logger._strip_base64_from_messages_sync(
        json.loads(json.dumps(strip_payload)))
    details = {
        "litellm_params": {"proxy_server_request": {
            "classifier_input": "secret", "model": "m",
            "originating_request_masked": "x", "other": 1}},
        "standard_logging_object": {
            "id": "log-1", "messages": [{"role": "user", "content": "secret prompt"}],
            "response": {"id": "r", "choices": [
                {"message": {"role": "assistant", "content": "secret answer"}}]},
            "classifier_input": "secret", "originating_request_masked": "secret",
            "other": {"keep": True}},
        "extra": "kept",
    }
    s["redact_same_object"] = (
        logger.redact_standard_logging_payload_from_model_call_details(details) is details)
    copy_details = {"litellm_params": {}}
    copy_result = redacting.redact_standard_logging_payload_from_model_call_details(copy_details)
    s["redact_copy_is_copy"] = copy_result is not copy_details
    s["redact_copy_equal"] = copy_result == copy_details
    redacted = redacting.redact_standard_logging_payload_from_model_call_details(
        json.loads(json.dumps(details)))
    slo = redacted["standard_logging_object"]
    s["redact_messages"] = [{"content": item.get("content"), "role": item.get("role")}
                            for item in slo["messages"]]
    s["redact_response"] = response_view(slo["response"])
    s["redact_slo_keys"] = sorted(slo)
    s["redact_proxy_request"] = redacted["litellm_params"]["proxy_server_request"]
    s["redact_extra"] = redacted["extra"]
    setattr(package, "standard_logging_payload_excluded_fields", ["messages", "response"])
    excluded = logger.redact_standard_logging_payload_from_model_call_details(
        json.loads(json.dumps(details)))
    s["redact_excluded_slo"] = excluded["standard_logging_object"]
    s["redact_excluded_proxy"] = excluded["litellm_params"]["proxy_server_request"]
    setattr(package, "standard_logging_payload_excluded_fields", ["messages"])
    combined = redacting.redact_standard_logging_payload_from_model_call_details(
        json.loads(json.dumps(details)))
    s["redact_combined_keys"] = sorted(combined["standard_logging_object"])
    s["redact_combined_response"] = response_view(
        combined["standard_logging_object"]["response"])
    setattr(package, "standard_logging_payload_excluded_fields", None)
    responses_details = {"standard_logging_object": {"id": "resp-1", "response": {
        "id": "resp", "output": [{"content": [{"text": "secret", "type": "text"}, "plain"]},
                                 5, {"content": "not-a-list"}]}}}
    responses_result = redacting.redact_standard_logging_payload_from_model_call_details(
        json.loads(json.dumps(responses_details)))
    s["redact_responses_api"] = responses_result["standard_logging_object"]["response"]
    try:
        redacting.redact_standard_logging_payload_from_model_call_details(
            {"standard_logging_object": ["not-a-dict"]})
        error_type = "none"
    except Exception as error:
        error_type = type(error).__name__
    s["stripped"] = asyncio.run(
        logger._strip_base64_from_messages(json.loads(json.dumps(strip_payload))))
    s["stripped_empty"] = asyncio.run(logger._strip_base64_from_messages({"model": "m"}))
    return {"shared": s, "error_type": error_type}

def batch_report(package):
    batch_cls = batch_class(package); s = {}
    default = batch_cls()
    s["defaults"] = {"flush_interval": default.flush_interval,
                     "batch_size": default.batch_size,
                     "max_queue_size": default.max_queue_size,
                     "flush_lock_none": default.flush_lock is None,
                     "last_flush_type": type(default.last_flush_time).__name__}
    custom = batch_cls(batch_size=2, flush_interval=1.5, max_queue_size=7)
    s["custom"] = {"flush_interval": custom.flush_interval, "batch_size": custom.batch_size,
                   "max_queue_size": custom.max_queue_size}
    s["class_vars"] = {
        "preserve_events_added_during_flush": batch_cls.preserve_events_added_during_flush,
        "default_max_queue_size": batch_cls.DEFAULT_MAX_QUEUE_SIZE}
    zero = batch_cls(batch_size=0, flush_interval=0)
    deviations = {"zero_batch_size": zero.batch_size, "zero_flush_interval": zero.flush_interval}
    setattr(package, "DEFAULT_BATCH_SIZE", 7)
    setattr(package, "DEFAULT_FLUSH_INTERVAL_SECONDS", 3)
    late = batch_cls()
    deviations["late_batch_size"] = late.batch_size
    deviations["late_flush_interval"] = late.flush_interval
    setattr(package, "DEFAULT_BATCH_SIZE", 512)
    setattr(package, "DEFAULT_FLUSH_INTERVAL_SECONDS", 5)
    async def run_all():
        results = {}
        records = []
        async def counting(*args, **kwargs):
            records.append(1)
        async def failing(*args, **kwargs):
            raise RuntimeError("boom")
        no_lock = batch_cls()
        no_lock.log_queue.extend([1, 2, 3])
        no_lock.async_send_batch = counting
        await no_lock.flush_queue()
        results["no_lock"] = {"queue": list(no_lock.log_queue), "sent": len(records)}
        ok = batch_cls(flush_lock=asyncio.Lock())
        ok.log_queue.extend([1, 2, 3])
        before = ok.last_flush_time
        start = len(records)
        ok.async_send_batch = counting
        await ok.flush_queue()
        results["send_ok"] = {"queue": list(ok.log_queue), "sent": len(records) - start,
                              "advanced": ok.last_flush_time >= before}
        await ok.flush_queue()
        results["send_ok"]["resend"] = {"queue": list(ok.log_queue),
                                        "sent": len(records) - start}
        raising = batch_cls(flush_lock=asyncio.Lock(), max_queue_size=2)
        raising.log_queue.extend([1, 2, 3, 4, 5])
        raising.async_send_batch = failing
        await raising.flush_queue()
        results["send_raises"] = {"queue": list(raising.log_queue)}
        under = batch_cls(flush_lock=asyncio.Lock())
        under.log_queue.extend([1, 2])
        under.async_send_batch = failing
        await under.flush_queue()
        results["send_under_cap"] = {"queue": list(under.log_queue)}
        preserving = batch_cls(flush_lock=asyncio.Lock())
        preserving.preserve_events_added_during_flush = True
        preserving.log_queue.extend(["a", "b"])
        async def adding(*args, **kwargs):
            preserving.log_queue.append("during")
        preserving.async_send_batch = adding
        await preserving.flush_queue()
        results["preserve"] = {"queue": list(preserving.log_queue)}
        clearing = batch_cls(flush_lock=asyncio.Lock())
        clearing.log_queue.extend(["a", "b"])
        async def adding_clear(*args, **kwargs):
            clearing.log_queue.append("during")
        clearing.async_send_batch = adding_clear
        await clearing.flush_queue()
        results["not_preserve"] = {"queue": list(clearing.log_queue)}
        periodic = batch_cls(flush_lock=asyncio.Lock(), flush_interval=0.01)
        periodic.log_queue.append("p"); periodic_sends = []
        async def periodic_send(*args, **kwargs): periodic_sends.append(1)
        periodic.async_send_batch = periodic_send
        periodic_task = asyncio.create_task(periodic.periodic_flush())
        for _ in range(500):
            if periodic_sends: break
            await asyncio.sleep(0.01)
        periodic_task.cancel()
        try: await periodic_task
        except asyncio.CancelledError: pass
        results["periodic"] = {"sent": len(periodic_sends), "queue": list(periodic.log_queue)}
        results["default_send"] = await batch_cls().async_send_batch()
        return results
    s.update(asyncio.run(run_all()))
    return {"shared": s, "deviations": deviations}

RUNNERS = {"surface": surface_report, "hooks": hooks_report, "helpers": helpers_report,
           "batch": batch_report}
GROUP = sys.argv[1]
print(json.dumps({side: RUNNERS[GROUP](package) for side, package in PACKAGES.items()}))
"""


def mapping(value: JsonValue) -> dict[str, JsonValue]:
    match value:
        case dict() as found:
            return found
        case str() | bool() | int() | float() | list() | None:
            raise AssertionError(value)


def listing(value: JsonValue) -> list[JsonValue]:
    match value:
        case list() as found:
            return found
        case str() | bool() | int() | float() | dict() | None:
            raise AssertionError(value)


def text(value: JsonValue) -> str:
    match value:
        case str() as found:
            return found
        case bool() | int() | float() | list() | dict() | None:
            raise AssertionError(value)


def dig(value: JsonValue, *path: str) -> JsonValue:
    for key in path:
        value = mapping(value)[key]
    return value


def run_probe(group: str) -> str:
    completed = subprocess.run(
        [sys.executable, "-c", PROBE, group],
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    return completed.stdout


@pytest.fixture(scope="module")
def surface_report() -> JsonValue:
    adapter = TypeAdapter(JsonValue)
    return adapter.validate_json(run_probe("surface"))


@pytest.fixture(scope="module")
def hooks_report() -> JsonValue:
    adapter = TypeAdapter(JsonValue)
    return adapter.validate_json(run_probe("hooks"))


@pytest.fixture(scope="module")
def helpers_report() -> JsonValue:
    adapter = TypeAdapter(JsonValue)
    return adapter.validate_json(run_probe("helpers"))


@pytest.fixture(scope="module")
def batch_report() -> JsonValue:
    adapter = TypeAdapter(JsonValue)
    return adapter.validate_json(run_probe("batch"))


def compare_methods(
    theirs: dict[str, JsonValue], ours: dict[str, JsonValue], *, keyword_init: bool
) -> None:
    assert set(ours) == set(theirs)
    for name in sorted(theirs):
        expected = mapping(theirs[name])
        found = mapping(ours[name])
        assert found["is_async"] == expected["is_async"], name
        expected_rows = [mapping(item) for item in listing(expected["params"])]
        found_rows = [mapping(item) for item in listing(found["params"])]
        if name in UNPACK_METHODS:
            assert [row["name"] for row in found_rows] == ["self", "params"], name
            assert found_rows[1]["kind"] == "VAR_KEYWORD", name
            expected_names = [text(row["name"]) for row in expected_rows]
            unpacked = sorted(text(item) for item in listing(found["unpack_keys"]))
            assert unpacked == sorted(item for item in expected_names if item != "self"), name
            continue
        if name == "__init__" and keyword_init:
            assert [row["name"] for row in found_rows] == [
                row["name"] for row in expected_rows
            ], name
            assert [row["default"] for row in found_rows] == [
                row["default"] for row in expected_rows
            ], name
            continue
        assert found["params"] == expected["params"], name


def test_logger_surface_methods(surface_report: JsonValue) -> None:
    theirs = mapping(dig(surface_report, "litellm"))
    ours = mapping(dig(surface_report, "betterlite"))
    our_methods = mapping(ours["custom_logger_methods"])
    their_methods = mapping(theirs["custom_logger_methods"])
    compare_methods(their_methods, our_methods, keyword_init=True)
    compare_methods(
        mapping(theirs["custom_batch_methods"]),
        mapping(ours["custom_batch_methods"]),
        keyword_init=False,
    )
    init_params = {
        text(mapping(item)["name"]): mapping(item)
        for item in listing(mapping(our_methods["__init__"])["params"])
    }
    assert init_params["turn_off_message_logging"]["kind"] == "KEYWORD_ONLY"
    assert init_params["message_logging"]["kind"] == "KEYWORD_ONLY"
    their_params = {
        text(mapping(item)["name"]): mapping(item)
        for item in listing(mapping(their_methods["__init__"])["params"])
    }
    assert their_params["turn_off_message_logging"]["kind"] == "POSITIONAL_OR_KEYWORD"
    assert their_params["message_logging"]["kind"] == "POSITIONAL_OR_KEYWORD"
    class_vars = mapping(ours["class_vars"])
    assert class_vars == mapping(theirs["class_vars"])
    assert class_vars["server_fulfilled_tool_names"] == []
    assert class_vars["enforces_request_content"] is False
    assert class_vars["preserve_events_added_during_flush"] is False
    assert class_vars["default_max_queue_size"] == MAX_QUEUE_DEFAULT


def test_logger_surface_reexports(surface_report: JsonValue) -> None:
    theirs = mapping(dig(surface_report, "litellm", "modules"))
    ours = mapping(dig(surface_report, "betterlite", "modules"))
    assert set(ours) == set(theirs)
    for suffix in sorted(ours):
        assert mapping(ours[suffix])["missing_in_litellm"] == [], suffix
        assert mapping(ours[suffix])["same_objects"] is True, suffix
        assert mapping(theirs[suffix])["all"] == [], suffix
    assert dig(ours, "integrations.custom_logger", "all") == CUSTOM_LOGGER_ALL
    assert dig(ours, "integrations.custom_batch_logger", "all") == CUSTOM_BATCH_ALL
    assert dig(ours, "types.integrations.custom_logger", "all") == CUSTOM_LOGGER_TYPES_ALL


def test_logger_surface_subclass(surface_report: JsonValue) -> None:
    theirs = mapping(dig(surface_report, "litellm", "subclass"))
    ours = mapping(dig(surface_report, "betterlite", "subclass"))
    assert ours == theirs
    assert ours["seen"] == ["model-one"]
    assert ours["filtered"] == ["keep-me"]


def test_logger_surface_types(surface_report: JsonValue) -> None:
    theirs = mapping(dig(surface_report, "litellm", "types"))
    ours = mapping(dig(surface_report, "betterlite", "types"))
    assert ours == theirs
    assert ours["is_internal"] == [False, True, True, True, True, True, False]
    assert ours["converted"] == [True, True, False, True, False, False]
    assert ours["prefixes"] == [
        "_code_interpreter_interception", "_compression_interception",
        "_headroom_interception", "_websearch_interception",
    ]
    init_params = listing(ours["init_params"])
    assert init_params[0] == {"turn_off_message_logging": False}
    assert init_params[1] == {"turn_off_message_logging": True}
    assert init_params[2] == {"turn_off_message_logging": None}
    assert mapping(ours["patch"])["optional_params"] == {}
    assert mapping(ours["patch"])["model"] is None
    assert ours["safety_error"] == ["AgenticLoopSafetyError", True]


def test_logger_hook_defaults(hooks_report: JsonValue) -> None:
    theirs = mapping(dig(hooks_report, "litellm"))
    ours = mapping(dig(hooks_report, "betterlite"))
    theirs_none = mapping(theirs["none"])
    ours_none = mapping(ours["none"])
    assert set(ours_none) == set(theirs_none) == NONE_HOOKS
    assert all(ours_none.values())
    assert ours["values"] == theirs["values"]
    values = mapping(ours["values"])
    assert values["agentic_gate"] == [False, {}]
    assert values["chat_gate"] == [False, {}]
    assert values["dataset_error_type"] == "NotImplementedError"
    assert values["dataset_error_text"] == "async_dataset_hook not implemented"
    assert values["env_vars"] == [[], []]
    assert values["filter_models"] == ["keep-a", "drop-b"]
    assert values["redacts_itself"] is False


def test_logger_hook_identities(hooks_report: JsonValue) -> None:
    values = mapping(dig(hooks_report, "betterlite", "values"))
    same_object = ["filter_deployments_same", "filter_models_same", "logging_hook_same",
                   "async_logging_hook_same", "agentic_post_same"]
    for field in same_object:
        assert values[field] is True, field
    assert values["streaming_iterator"] == [1, 2]
    prompt = listing(values["prompt"])
    assert prompt == values["async_prompt"]
    assert prompt[0] == "m"
    assert prompt[1] == [{"role": "user", "content": "hi"}]
    assert prompt[2] == {"temperature": 0.5}
    plan = mapping(values["agentic_plan"])
    assert values["agentic_plan"] == values["chat_plan"]
    assert plan["run_agentic_loop"] is False
    assert plan["terminate"] is False
    assert plan["request_patch"] is None
    assert plan["metadata"] == {}


def test_logger_hook_events(hooks_report: JsonValue) -> None:
    theirs = mapping(dig(hooks_report, "litellm", "events"))
    ours = mapping(dig(hooks_report, "betterlite", "events"))
    assert ours["input_calls"] == theirs["input_calls"] == ONE
    assert ours["input_same_kwargs"] is True
    assert theirs["input_same_kwargs"] is True
    assert ours["input_model"] == theirs["input_model"] == "new-model"
    assert ours["input_type"] == theirs["input_type"] == "pre_api_call"
    assert text(ours["input_verbose"]) == text(theirs["input_verbose"])
    assert text(ours["input_verbose"]).startswith("Custom Logger - model call details: ")
    assert ours["event_calls"] == theirs["event_calls"] == EVENT_ARG_COUNT
    assert ours["event_type"] == theirs["event_type"] == "post_api_call"
    assert ours["event_verbose_count"] == theirs["event_verbose_count"] == ZERO
    assert ours["async_input_calls"] == theirs["async_input_calls"] == ONE
    assert ours["async_input_same_kwargs"] is True
    assert theirs["async_input_same_kwargs"] is True
    assert ours["async_input_type"] == theirs["async_input_type"] == "pre_api_call"
    assert text(ours["async_input_verbose"]) == text(theirs["async_input_verbose"])
    assert text(ours["async_input_verbose"]).startswith(
        "Custom Logger - model call details: ")
    assert ours["async_event_calls"] == theirs["async_event_calls"] == EVENT_ARG_COUNT
    assert ours["async_event_type"] == theirs["async_event_type"] == "post_api_call"
    assert ours["async_event_verbose_count"] == theirs["async_event_verbose_count"] == ZERO
    assert ours["input_error"] == "raised"
    assert theirs["input_error"] == "swallowed"
    assert ours["input_verbose_count"] == ONE
    assert theirs["input_verbose_count"] == TWO
    assert ours["input_error_message"] is None
    assert text(theirs["input_error_message"]).startswith("Custom Logger Error - ")
    assert "RuntimeError: boom" in text(theirs["input_error_message"])
    assert ours["event_error"] == "raised"
    assert theirs["event_error"] == "swallowed"
    assert ours["event_error_verbose"] == ZERO
    assert theirs["event_error_verbose"] == ONE
    assert ours["event_error_message"] is None
    assert text(theirs["event_error_message"]).startswith("Custom Logger Error - ")
    assert ours["async_input_error"] == "raised"
    assert theirs["async_input_error"] == "swallowed"
    assert ours["async_input_error_message"] is None
    assert text(theirs["async_input_error_message"]).startswith("Custom Logger Error - ")
    assert "RuntimeError: boom" in text(theirs["async_input_error_message"])
    assert ours["async_event_error"] == "raised"
    assert theirs["async_event_error"] == "swallowed"
    assert ours["async_event_error_verbose"] == ZERO
    assert theirs["async_event_error_verbose"] == ONE
    assert ours["async_event_error_message"] is None
    assert text(theirs["async_event_error_message"]).startswith("Custom Logger Error - ")


def test_logger_hook_init(hooks_report: JsonValue) -> None:
    theirs = mapping(dig(hooks_report, "litellm", "init"))
    ours = mapping(dig(hooks_report, "betterlite", "init"))
    expected = {
        "default_message_logging": True, "default_turn_off": False,
        "flagged_message_logging": True, "flagged_turn_off": True,
        "unflagged_message_logging": False, "both_message_logging": False,
        "both_turn_off": True,
    }
    assert {field: ours[field] for field in expected} == expected
    assert ours["positional_error"] == "TypeError"
    assert theirs["positional_error"] == "none"


def test_logger_helper_truncate(helpers_report: JsonValue) -> None:
    theirs = mapping(dig(helpers_report, "litellm", "shared"))
    ours = mapping(dig(helpers_report, "betterlite", "shared"))
    assert ours == theirs
    long_text = text(ours["truncate_text_long"])
    assert long_text.endswith(TRUNCATION_SUFFIX)
    assert len(long_text) == MAX_STR_LENGTH + len(TRUNCATION_SUFFIX)
    assert ours["truncate_text_short"] == "short"
    assert ours["truncate_field_none"] is None
    assert ours["truncate_field_short"] is None
    assert text(ours["truncate_field_list"]).endswith(TRUNCATION_SUFFIX)
    assert listing(ours["truncate_exact"]) == ["x" * MAX_STR_LENGTH, None]
    assert ours["truncated_input_unchanged"] is True
    assert ours["metadata_fields"] == [None, "metadata", "litellm_metadata", "litellm_metadata"]
    assert ours["keep_content"] == [
        True, False, False, True, True, False, True, True, True,
    ]
    truncated = mapping(ours["truncated"])
    assert text(truncated["error_str"]).endswith(TRUNCATION_SUFFIX)
    assert text(truncated["messages"]).endswith(TRUNCATION_SUFFIX)
    assert truncated["response"] == "short"
    assert truncated["extra"] == {"keep": True}


def test_logger_helper_redact(helpers_report: JsonValue) -> None:
    ours = mapping(dig(helpers_report, "betterlite", "shared"))
    assert ours["redact_same_object"] is True
    assert ours["redact_copy_is_copy"] is True
    assert ours["redact_copy_equal"] is True
    assert listing(ours["redact_messages"])[0] == {
        "content": REDACTED_CONTENT, "role": "assistant"}
    response = mapping(ours["redact_response"])
    assert response["has_choices"] is True
    assert response["content"] == REDACTED_CONTENT
    assert ours["redact_slo_keys"] == ["id", "messages", "other", "response"]
    assert ours["redact_proxy_request"] == {"model": "m", "other": 1}
    assert ours["redact_extra"] == "kept"
    assert ours["redact_combined_keys"] == ["id", "other", "response"]
    assert mapping(ours["redact_combined_response"])["has_choices"] is True


def test_logger_helper_excluded(helpers_report: JsonValue) -> None:
    theirs = mapping(dig(helpers_report, "litellm"))
    ours = mapping(dig(helpers_report, "betterlite"))
    assert ours["shared"] == theirs["shared"]
    shared = mapping(ours["shared"])
    assert sorted(mapping(shared["redact_excluded_slo"])) == [
        "classifier_input", "id", "originating_request_masked", "other"]
    assert shared["redact_excluded_proxy"] == {
        "classifier_input": "secret", "model": "m",
        "originating_request_masked": "x", "other": 1}
    assert ours["error_type"] == "TypeError"
    assert theirs["error_type"] == "AttributeError"


def test_logger_helper_base64(helpers_report: JsonValue) -> None:
    ours = mapping(dig(helpers_report, "betterlite", "shared"))
    values = listing(ours["redact_base64"])
    assert values[0] == BASE64_MARKER
    assert values[1] == {"url": BASE64_MARKER}
    assert listing(values[2])[0] == BASE64_MARKER
    assert values[3] == "plain"
    assert values[4] == SMALL_NUMBER
    assert values[5] is None
    assert values[6] is True
    assert ours["redact_depth"] == [[[MAX_DEPTH_MARKER]]]
    processed = listing(ours["processed"])
    expected_text = "see " + BASE64_MARKER
    assert processed[0] == {"role": "user", "content": [{"type": "text", "text": expected_text}]}
    assert processed[1] == {"role": "assistant", "content": "plain " + BASE64_MARKER}
    assert processed[2] == {"role": "system", "content": ZERO}
    assert ours["stripped"] == ours["stripped_sync"]
    assert ours["stripped"] == {"model": "m", "messages": processed}
    assert ours["stripped_empty"] == {"model": "m"}
    responses = mapping(ours["redact_responses_api"])
    assert responses["id"] == "resp"
    assert dig(responses, "output") == [
        {"content": [{"text": REDACTED_CONTENT, "type": "text"}, "plain"]},
        SMALL_NUMBER,
        {"content": "not-a-list"},
    ]


def test_batch_logger_defaults(batch_report: JsonValue) -> None:
    theirs = mapping(dig(batch_report, "litellm"))
    ours = mapping(dig(batch_report, "betterlite"))
    assert mapping(ours["shared"]) == mapping(theirs["shared"])
    shared = mapping(ours["shared"])
    defaults = mapping(shared["defaults"])
    assert defaults["batch_size"] == BATCH_SIZE_DEFAULT
    assert defaults["flush_interval"] == FLUSH_INTERVAL_DEFAULT
    assert defaults["max_queue_size"] == MAX_QUEUE_DEFAULT
    assert defaults["flush_lock_none"] is True
    assert defaults["last_flush_type"] == "float"
    custom = mapping(shared["custom"])
    assert custom["batch_size"] == CUSTOM_BATCH_SIZE
    assert custom["flush_interval"] == CUSTOM_FLUSH_INTERVAL
    assert custom["max_queue_size"] == CUSTOM_MAX_QUEUE
    class_vars = mapping(shared["class_vars"])
    assert class_vars["preserve_events_added_during_flush"] is False
    assert class_vars["default_max_queue_size"] == MAX_QUEUE_DEFAULT
    theirs_deviations = mapping(theirs["deviations"])
    ours_deviations = mapping(ours["deviations"])
    assert ours_deviations["zero_batch_size"] == ZERO
    assert ours_deviations["zero_flush_interval"] == ZERO
    assert ours_deviations["late_batch_size"] == BATCH_SIZE_DEFAULT
    assert ours_deviations["late_flush_interval"] == FLUSH_INTERVAL_DEFAULT
    assert theirs_deviations["zero_batch_size"] == BATCH_SIZE_DEFAULT
    assert theirs_deviations["zero_flush_interval"] == FLUSH_INTERVAL_DEFAULT
    assert theirs_deviations["late_batch_size"] == LATE_BATCH_SIZE
    assert theirs_deviations["late_flush_interval"] == LATE_FLUSH_INTERVAL


def test_batch_logger_no_lock(batch_report: JsonValue) -> None:
    shared = mapping(dig(batch_report, "betterlite", "shared"))
    no_lock = mapping(shared["no_lock"])
    assert no_lock["queue"] == [1, 2, 3]
    assert no_lock["sent"] == ZERO


def test_batch_logger_send_ok(batch_report: JsonValue) -> None:
    shared = mapping(dig(batch_report, "betterlite", "shared"))
    send_ok = mapping(shared["send_ok"])
    assert send_ok["queue"] == []
    assert send_ok["sent"] == ONE
    assert send_ok["advanced"] is True
    assert mapping(send_ok["resend"]) == {"queue": [], "sent": ONE}


def test_batch_logger_raises(batch_report: JsonValue) -> None:
    shared = mapping(dig(batch_report, "betterlite", "shared"))
    assert mapping(shared["send_raises"])["queue"] == [4, 5]
    assert mapping(shared["send_under_cap"])["queue"] == [1, 2]
    assert shared["default_send"] is None


def test_batch_logger_periodic(batch_report: JsonValue) -> None:
    shared = mapping(dig(batch_report, "betterlite", "shared"))
    periodic = mapping(shared["periodic"])
    assert periodic["sent"] == ONE
    assert periodic["queue"] == []


def test_batch_logger_preserve(batch_report: JsonValue) -> None:
    shared = mapping(dig(batch_report, "betterlite", "shared"))
    assert mapping(shared["preserve"])["queue"] == ["during"]
    assert mapping(shared["not_preserve"])["queue"] == []
