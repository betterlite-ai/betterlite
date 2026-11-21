import subprocess
import sys
from typing import Final, TypeAlias

import pytest
from pydantic import JsonValue, TypeAdapter

# custom_sso_handler.py is covered but not built: nothing calls custom_sso_handler at the pin (B063)
PROBE: Final = """
import asyncio; import importlib; import inspect; import json; import logging; import os; import sys
os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"; import betterlite
import litellm; import httpx
SIDES = {
    "litellm": {"prompt": "litellm.integrations.custom_prompt_management",
                "base": "litellm.integrations.prompt_management_base",
                "spec": "litellm.types.prompts.init_prompts",
                "secret": "litellm.integrations.custom_secret_manager",
                "secret_base": "litellm.secret_managers.base_secret_manager",
                "kms": "litellm.types.secret_managers.main",
                "logger": "litellm.integrations.custom_logger"},
    "betterlite": {"prompt": "betterlite.integrations.custom_prompt_management",
                   "base": "betterlite._integrations.prompt_management_base",
                   "spec": "betterlite.types.prompts.init_prompts",
                   "secret": "betterlite.integrations.custom_secret_manager",
                   "secret_base": "betterlite.secret_managers.base_secret_manager",
                   "kms": "betterlite.types.secret_managers.main",
                   "logger": "betterlite.integrations.custom_logger"},
}
EXPORTS = {"prompt": ["AllMessageValues", "CustomLogger", "CustomPromptManagement",
                      "PromptManagementBase", "PromptManagementClient", "PromptSpec",
                      "StandardCallbackDynamicParams"],
           "secret": ["BaseSecretManager", "CustomSecretManager"],
           "secret_base": ["BaseSecretManager", "raise_if_unsafe_secret_name"]}
RECORDS = []
class Collect(logging.Handler):
    def emit(self, record):
        RECORDS.append([record.levelname, record.exc_info is not None, record.getMessage()])
LOG = logging.getLogger("LiteLLM"); LOG.addHandler(Collect()); LOG.setLevel(logging.DEBUG)
def modules(side):
    return {key: importlib.import_module(name) for key, name in SIDES[side].items()}
def param_row(item):
    default = "required"
    if item.default is not inspect.Parameter.empty:
        default = repr(item.default)
    return [item.name, item.kind.name, default]
def method_rows(cls):
    rows = {}
    for name, member in inspect.getmembers(cls, inspect.isfunction):
        if name.startswith("__") and name != "__init__":
            continue
        rows[name] = [inspect.iscoroutinefunction(member),
                      [param_row(item) for item in inspect.signature(member).parameters.values()]]
    return rows
def outcome(call):
    try:
        return ["ok", call()]
    except Exception as error:
        return [type(error).__name__, str(error)]
def exports(side, found):
    return {key: [hasattr(found[key], name) for name in names] for key, names in EXPORTS.items()}
MESSAGES = [{"role": "user", "content": "hi"}]
def template(prompt_id):
    if prompt_id == "missing":
        return {"prompt_id": prompt_id}
    if prompt_id == "bad":
        return {"prompt_id": prompt_id, "prompt_template": None, "prompt_template_model": None,
                "prompt_template_optional_params": None, "completed_messages": None}
    params = None if prompt_id == "no-params" else {"temperature": 0.1}
    return {"prompt_id": prompt_id,
            "prompt_template": [{"role": "system", "content": "sys " + str(prompt_id)}],
            "prompt_template_model": "tmpl-model" if prompt_id == "with-model" else None,
            "prompt_template_optional_params": params, "completed_messages": None}
def templated_class(base, seen):
    class Templated(base):
        @property
        def integration_name(self):
            return "templ"
        def should_run_prompt_management(self, prompt_id, prompt_spec, dynamic_callback_params):
            return prompt_id != "skip"
        def _compile_prompt_helper(self, prompt_id, prompt_spec, prompt_variables,
                                   dynamic_callback_params, prompt_label=None, prompt_version=None):
            seen.append(["sync", prompt_id, prompt_spec is not None, prompt_variables,
                         prompt_label, prompt_version])
            return template(prompt_id)
        async def async_compile_prompt_helper(self, prompt_id, prompt_variables,
                                              dynamic_callback_params, prompt_spec=None,
                                              prompt_label=None, prompt_version=None):
            seen.append(["async", prompt_id, prompt_spec is not None, prompt_variables,
                         prompt_label, prompt_version])
            return template(prompt_id)
    return Templated
def spec(found, model, params):
    return found["spec"].PromptSpec(prompt_id="p1", litellm_params={
        "prompt_integration": "custom", "ignore_prompt_manager_model": model,
        "ignore_prompt_manager_optional_params": params})
def base_calls(found):
    seen = []; manager = templated_class(found["base"].PromptManagementBase, seen)(); r = {}
    def call(prompt_id, **extra):
        return outcome(lambda: list(manager.get_chat_completion_prompt(
            "templ/gpt-4o", list(MESSAGES), {"max_tokens": 5}, prompt_id, {"v": 1}, {}, **extra)))
    def async_call(prompt_id, **extra):
        return outcome(lambda: list(asyncio.run(manager.async_get_chat_completion_prompt(
            "templ/gpt-4o", list(MESSAGES), {"max_tokens": 5}, prompt_id, {"v": 1}, {}, None,
            **extra))))
    both = spec(found, True, True)
    r["sync_id"] = call("p1"); r["sync_no_id"] = call(None); r["sync_skipped"] = call("skip")
    r["sync_missing"] = call("missing"); r["sync_bad"] = call("bad")
    r["sync_with_model"] = call("with-model"); r["sync_no_params"] = call("no-params")
    r["sync_ignore_model"] = call("p1", ignore_prompt_manager_model=True)
    r["sync_ignore_params"] = call("p1", ignore_prompt_manager_optional_params=True)
    r["sync_spec"] = call("p1", prompt_spec=both)
    r["sync_label"] = call("p1", prompt_label="prod", prompt_version=3)
    r["async_id"] = async_call("p1"); r["async_no_id"] = async_call(None)
    r["async_skipped"] = async_call("skip"); r["async_missing"] = async_call("missing")
    r["async_spec"] = async_call("p1", prompt_spec=both, prompt_label="dev", prompt_version=2)
    r["seen"] = list(seen)
    template_none = {"prompt_id": "x", "prompt_template": [], "prompt_template_model": None,
                     "prompt_template_optional_params": None, "completed_messages": None}
    done = dict(template_none, completed_messages=[{"role": "system", "content": "done"}],
                prompt_template_model="picked", prompt_template_optional_params={"top_p": 1})
    r["post_none"] = list(manager.post_compile_prompt_processing(
        template_none, list(MESSAGES), {"a": 1}, "templ/m"))
    r["post_done"] = list(manager.post_compile_prompt_processing(done, [], {"a": 1}, "m"))
    r["post_done_ignored"] = list(manager.post_compile_prompt_processing(
        done, [], {"a": 1}, "templ/m", True, True))
    r["post_other_prefix"] = list(manager.post_compile_prompt_processing(
        template_none, [], {}, "other/m"))
    r["merge"] = manager.merge_messages([{"role": "system", "content": "a"}], list(MESSAGES))
    r["compile"] = manager.compile_prompt("p1", None, list(MESSAGES), {})
    r["async_compile_none"] = asyncio.run(manager.async_compile_prompt(None, None, [], {}))
    r["model_from_prompt"] = [manager._get_model_from_prompt(template("with-model"), "templ/x"),
                              manager._get_model_from_prompt(template("p1"), "templ/templ/x")]
    return r
def flag_calls(found):
    resolve = found["base"].resolve_prompt_manager_ignore_flags
    specs = [None, spec(found, True, False), spec(found, False, True), spec(found, None, None)]
    return {"flags": [list(resolve(item, model, params)) for item in specs
                      for model in (None, False, True) for params in (None, False, True)]}
def custom_calls(found):
    cls = found["prompt"].CustomPromptManagement; base = found["base"].PromptManagementBase
    default = cls(); positional = cls(True, None)
    keywords = cls(ignore_prompt_manager_model=None, ignore_prompt_manager_optional_params=True,
                   extra=1)
    class Mine(cls):
        def __init__(self):
            super().__init__(ignore_prompt_manager_model=True)
        def get_chat_completion_prompt(self, model, messages, non_default_params, prompt_id,
                                       prompt_variables, dynamic_callback_params, **rest):
            return ("mine/" + model, [{"role": "system", "content": str(prompt_id)}] + messages,
                    {**non_default_params, "temperature": 0.2})
    mine = Mine(); helper_args = ("p1", None, None, {})
    return {
        "default_flags": [default.ignore_prompt_manager_model,
                          default.ignore_prompt_manager_optional_params],
        "positional_flags": [positional.ignore_prompt_manager_model,
                             positional.ignore_prompt_manager_optional_params],
        "keyword_flags": [keywords.ignore_prompt_manager_model,
                          keywords.ignore_prompt_manager_optional_params],
        "has_message_logging": [hasattr(default, "message_logging"),
                                hasattr(default, "turn_off_message_logging")],
        "integration_name": default.integration_name,
        "should_run": [default.should_run_prompt_management(None, None, {}),
                       default.should_run_prompt_management("p1", None, {})],
        "compile_helper": outcome(lambda: default._compile_prompt_helper(*helper_args)),
        "async_compile_helper": outcome(
            lambda: asyncio.run(default.async_compile_prompt_helper("p1", None, {}))),
        "compile_prompt": outcome(lambda: default.compile_prompt("p1", None, list(MESSAGES), {})),
        "passthrough": list(default.get_chat_completion_prompt(
            "m", list(MESSAGES), {"t": 1}, "p1", None, {})),
        "async_passthrough": list(asyncio.run(default.async_get_chat_completion_prompt(
            "m", list(MESSAGES), {"t": 1}, "p1", None, {}, None))),
        "mine": list(mine.get_chat_completion_prompt("gpt", list(MESSAGES), {"t": 1}, "p9",
                                                     None, {})),
        "mine_async": list(asyncio.run(mine.async_get_chat_completion_prompt(
            "gpt", list(MESSAGES), {"t": 1}, "p9", None, {}, None))),
        "mine_flags": [mine.ignore_prompt_manager_model,
                       mine.ignore_prompt_manager_optional_params],
        "mine_kinds": [isinstance(mine, base), isinstance(mine, found["logger"].CustomLogger)],
    }
def prompt_report(side):
    found = modules(side); cls = found["prompt"].CustomPromptManagement
    base = found["base"].PromptManagementBase
    class Partial(base):
        @property
        def integration_name(self):
            return "partial"
    r = {"custom_methods": method_rows(cls), "base_methods": method_rows(base),
         "base_abstract": sorted(base.__abstractmethods__),
         "custom_abstract": sorted(cls.__abstractmethods__),
         "partial": outcome(lambda: type(Partial()).__name__)[0],
         "properties": [isinstance(inspect.getattr_static(cls, "integration_name"), property),
                        isinstance(inspect.getattr_static(base, "integration_name"), property)],
         "exports": exports(side, found)}
    r.update(base_calls(found)); r.update(flag_calls(found)); r.update(custom_calls(found))
    return r
def store_class(custom, calls):
    class Store(custom):
        def __init__(self, data):
            super().__init__(secret_manager_name="store"); self.data = data
        async def async_read_secret(self, secret_name, optional_params=None, timeout=None):
            calls.append(["read", secret_name, optional_params, timeout])
            return self.data.get(secret_name)
        def sync_read_secret(self, secret_name, optional_params=None, timeout=None):
            calls.append(["sync_read", secret_name, optional_params, timeout])
            return self.data.get(secret_name)
        async def async_write_secret(self, secret_name, secret_value, description=None,
                                     optional_params=None, timeout=None, tags=None):
            calls.append(["write", secret_name, secret_value, description, optional_params,
                          timeout, tags])
            if secret_name != "unverified":
                self.data[secret_name] = secret_value
            return {"name": secret_name, "status": "written"}
        async def async_delete_secret(self, secret_name, recovery_window_in_days=7,
                                      optional_params=None, timeout=None):
            calls.append(["delete", secret_name, recovery_window_in_days, optional_params,
                          timeout])
            self.data.pop(secret_name, None)
            return {"name": secret_name, "deleted": True}
    return Store
def read_only_class(custom, error):
    class ReadOnly(custom):
        async def async_read_secret(self, secret_name, optional_params=None, timeout=None):
            if error is not None:
                raise error
            return "value-" + secret_name
        def sync_read_secret(self, secret_name, optional_params=None, timeout=None):
            return "sync-" + secret_name
    return ReadOnly
def rotate(manager, *args):
    RECORDS.clear()
    result = outcome(lambda: asyncio.run(manager.async_rotate_secret(*args)))
    return result, [record[:2] for record in RECORDS], [record[2] for record in RECORDS]
def http_errors():
    request = httpx.Request("GET", "https://vault.test/v1/secret")
    response = httpx.Response(503, text="vault down", request=request)
    return (httpx.TimeoutException("slow vault"),
            httpx.HTTPStatusError("bad status", request=request, response=response))
def store_calls(found):
    calls = []; custom = found["secret"].CustomSecretManager
    store = store_class(custom, calls)({"a": "1", "keep": "k"}); r = {}
    r["sync_read"] = [store.sync_read_secret("a"), store.sync_read_secret("none", {"x": 1}, 2.5)]
    r["async_read"] = [asyncio.run(store.async_read_secret("a")),
                       asyncio.run(store.async_read_secret("none"))]
    r["write"] = asyncio.run(store.async_write_secret("b", "2", "about b", {"x": 1}, 3.0,
                                                      {"Env": "Prod"}))
    r["delete"] = asyncio.run(store.async_delete_secret("b"))
    r["rotated"], r["rotated_levels"], _ = rotate(store, "a", "a2", "new", {"region": "x"}, 5.0)
    r["rotate_missing"], r["missing_levels"], r["missing_messages"] = rotate(store, "zz", "n", "v")
    r["rotate_unverified"], r["unverified_levels"], _ = rotate(store, "keep", "unverified", "v")
    r["store_calls"] = calls; r["store_data"] = store.data
    return r
def default_calls(found):
    custom = found["secret"].CustomSecretManager; RECORDS.clear()
    plain = read_only_class(custom, None)(); named = read_only_class(custom, None)(
        secret_manager_name="vault", extra=1)
    timeout, status = http_errors(); r = {"init_records": [record[:2] for record in RECORDS]}
    r["names"] = [plain.secret_manager_name, named.secret_manager_name]
    r["reprs"] = [repr(plain), repr(named)]
    r["default_write"] = outcome(lambda: asyncio.run(named.async_write_secret("s", "v")))
    r["default_delete"] = outcome(lambda: asyncio.run(plain.async_delete_secret("s")))
    r["rotate_no_write"], r["no_write_levels"], _ = rotate(named, "s", "t", "v")
    r["validate"] = plain.validate_environment()
    r["health"] = [asyncio.run(plain.async_health_check()),
                   asyncio.run(plain.async_health_check(timeout=1.0))]
    r["httpx_timeout"], r["timeout_levels"], _ = rotate(
        read_only_class(custom, timeout)(), "s", "t", "v")
    r["httpx_status"], r["status_levels"], _ = rotate(
        read_only_class(custom, status)(), "s", "t", "v")
    return r
UNSAFE = ["../x", "x/..", "..", "a/../b", "release-1.0..2", "..x", "x..", "ok/name", "a\\x00b",
          "a\\x1fb", "a\\x7fb", "a\\x85b", "a\\x9fb", "a\\xa0b", "a\\u2028b", "a\\u2029b", "",
          "plain name"]
def secret_report(side):
    found = modules(side); custom = found["secret"].CustomSecretManager
    base = found["secret_base"].BaseSecretManager
    class HalfCustom(custom):
        async def async_read_secret(self, secret_name, optional_params=None, timeout=None):
            return None
    class HalfBase(base):
        async def async_read_secret(self, secret_name, optional_params=None, timeout=None):
            return None
        def sync_read_secret(self, secret_name, optional_params=None, timeout=None):
            return None
    check = found["secret_base"].raise_if_unsafe_secret_name
    r = {"custom_methods": method_rows(custom), "base_methods": method_rows(base),
         "base_abstract": sorted(base.__abstractmethods__),
         "custom_abstract": sorted(custom.__abstractmethods__),
         "half_custom": outcome(lambda: type(HalfCustom()).__name__)[0],
         "half_base": outcome(lambda: type(HalfBase()).__name__)[0],
         "is_base": issubclass(custom, base), "exports": exports(side, found),
         "kms_custom": found["kms"].KeyManagementSystem.CUSTOM.value,
         "unsafe": [outcome(lambda: check(name))[0] for name in UNSAFE],
         "unsafe_message": outcome(lambda: check("../x"))}
    r.update(store_calls(found)); r.update(default_calls(found))
    return r
RUNNERS = {"prompt": prompt_report, "secret": secret_report}
print(json.dumps({group: {side: run(side) for side in SIDES} for group, run in RUNNERS.items()}))
"""
Reports: TypeAlias = dict[str, dict[str, dict[str, JsonValue]]]
REPORT: Final = TypeAdapter(Reports)
USER: Final[JsonValue] = {"role": "user", "content": "hi"}
TEMPLATE: Final[JsonValue] = {"role": "system", "content": "sys p1"}
NONE_TEMPLATE: Final[JsonValue] = {"role": "system", "content": "sys None"}
DONE: Final[JsonValue] = [{"role": "system", "content": "done"}]
MERGED: Final[JsonValue] = [TEMPLATE, USER]
BASE_PARAMS: Final[JsonValue] = {"max_tokens": 5}
TEMPLATE_PARAMS: Final[JsonValue] = {"max_tokens": 5, "temperature": 0.1}
COMPILED: Final[JsonValue] = ["ok", ["gpt-4o", MERGED, TEMPLATE_PARAMS]]
IGNORED_SPEC: Final[JsonValue] = ["ok", ["templ/gpt-4o", MERGED, BASE_PARAMS]]
UNTOUCHED: Final[JsonValue] = ["ok", ["templ/gpt-4o", [USER], BASE_PARAMS]]
MISSING: Final[JsonValue] = [
    "ValueError",
    "Error compiling prompt: 'prompt_template'. Prompt id=missing",
]
BAD_TEMPLATE: Final = (
    "Error compiling prompt: unsupported operand type(s) for +: 'NoneType' and 'list'."
    " Prompt id=bad"
)
NO_HELPER: Final[JsonValue] = [
    "NotImplementedError",
    "Custom prompt management does not support compile prompt helper",
]
NO_ASYNC_HELPER: Final = "Custom prompt management does not support async compile prompt helper"
UNSET_FLAGS: Final[list[JsonValue]] = [
    [model, params] for model in (False, False, True) for params in (False, False, True)
]
MODEL_FLAGS: Final[list[JsonValue]] = [
    [True, params] for _ in range(3) for params in (False, False, True)
]
PARAM_FLAGS: Final[list[JsonValue]] = [
    [model, True] for model in (False, False, True) for _ in range(3)
]
SEEN: Final[JsonValue] = [
    ["sync", "p1", False, {"v": 1}, None, None],
    ["sync", "missing", False, {"v": 1}, None, None],
    ["sync", "bad", False, {"v": 1}, None, None],
    ["sync", "with-model", False, {"v": 1}, None, None],
    ["sync", "no-params", False, {"v": 1}, None, None],
    ["sync", "p1", False, {"v": 1}, None, None],
    ["sync", "p1", False, {"v": 1}, None, None],
    ["sync", "p1", False, {"v": 1}, None, None],
    ["sync", "p1", False, {"v": 1}, "prod", 3],
    ["async", "p1", False, {"v": 1}, None, None],
    ["async", None, False, {"v": 1}, None, None],
    ["async", "missing", False, {"v": 1}, None, None],
    ["async", "p1", True, {"v": 1}, "dev", 2],
]
PROMPT_EXPECTED: Final[dict[str, JsonValue]] = {
    "base_abstract": [
        "_compile_prompt_helper",
        "async_compile_prompt_helper",
        "integration_name",
        "should_run_prompt_management",
    ],
    "custom_abstract": [],
    "partial": "TypeError",
    "properties": [True, True],
    "exports": {
        "prompt": [True, True, True, True, True, True, True],
        "secret": [True, True],
        "secret_base": [True, True],
    },
    "sync_id": COMPILED,
    "sync_label": COMPILED,
    "async_id": COMPILED,
    "sync_no_params": [
        "ok",
        ["gpt-4o", [{"role": "system", "content": "sys no-params"}, USER], BASE_PARAMS],
    ],
    "sync_with_model": [
        "ok",
        ["tmpl-model", [{"role": "system", "content": "sys with-model"}, USER], TEMPLATE_PARAMS],
    ],
    "sync_ignore_model": ["ok", ["templ/gpt-4o", MERGED, TEMPLATE_PARAMS]],
    "sync_ignore_params": ["ok", ["gpt-4o", MERGED, BASE_PARAMS]],
    "sync_spec": IGNORED_SPEC,
    "async_spec": IGNORED_SPEC,
    "sync_no_id": UNTOUCHED,
    "sync_skipped": UNTOUCHED,
    "async_skipped": UNTOUCHED,
    "async_no_id": ["ok", ["gpt-4o", [NONE_TEMPLATE, USER], TEMPLATE_PARAMS]],
    "sync_missing": MISSING,
    "async_missing": MISSING,
    "sync_bad": ["ValueError", BAD_TEMPLATE],
    "seen": SEEN,
    "post_none": ["m", [USER], {"a": 1}],
    "post_done": ["picked", DONE, {"a": 1, "top_p": 1}],
    "post_done_ignored": ["templ/m", DONE, {"a": 1}],
    "post_other_prefix": ["other/m", [], {}],
    "merge": [{"role": "system", "content": "a"}, USER],
    "model_from_prompt": ["tmpl-model", "x"],
    "compile": {
        "prompt_id": "p1",
        "prompt_template": [TEMPLATE],
        "prompt_template_model": None,
        "prompt_template_optional_params": {"temperature": 0.1},
        "completed_messages": MERGED,
    },
    "async_compile_none": {
        "prompt_id": None,
        "prompt_template": [NONE_TEMPLATE],
        "prompt_template_model": None,
        "prompt_template_optional_params": {"temperature": 0.1},
        "completed_messages": [NONE_TEMPLATE],
    },
    "flags": [*UNSET_FLAGS, *MODEL_FLAGS, *PARAM_FLAGS, *UNSET_FLAGS],
    "default_flags": [False, False],
    "positional_flags": [True, None],
    "keyword_flags": [None, True],
    "has_message_logging": [False, False],
    "integration_name": "custom-prompt-management",
    "should_run": [True, True],
    "compile_helper": NO_HELPER,
    "compile_prompt": NO_HELPER,
    "async_compile_helper": ["NotImplementedError", NO_ASYNC_HELPER],
    "passthrough": ["m", [USER], {"t": 1}],
    "async_passthrough": ["m", [USER], {"t": 1}],
    "mine": ["mine/gpt", [{"role": "system", "content": "p9"}, USER], {"t": 1, "temperature": 0.2}],
    "mine_async": ["gpt", [USER], {"t": 1}],
    "mine_flags": [True, False],
    "mine_kinds": [True, True],
}
NOT_FOUND: Final = "Current secret zz not found"
ROTATE_LOG: Final = "Error rotating secret in AWS Secrets Manager"
ERROR_LOGGED: Final[JsonValue] = [["ERROR", True]]
NO_WRITE: Final[JsonValue] = [
    "NotImplementedError",
    (
        "Write operations are not implemented for vault. "
        "Override async_write_secret() to add write support."
    ),
]
REGION: Final[JsonValue] = {"region": "x"}
STORE_CALLS: Final[JsonValue] = [
    ["sync_read", "a", None, None],
    ["sync_read", "none", {"x": 1}, 2.5],
    ["read", "a", None, None],
    ["read", "none", None, None],
    ["write", "b", "2", "about b", {"x": 1}, 3.0, {"Env": "Prod"}],
    ["delete", "b", 7, None, None],
    ["read", "a", REGION, 5.0],
    ["write", "a2", "new", "Rotated from a", REGION, 5.0, None],
    ["read", "a2", REGION, 5.0],
    ["delete", "a", 7, REGION, 5.0],
    ["read", "zz", None, None],
    ["read", "keep", None, None],
    ["write", "unverified", "v", "Rotated from keep", None, None, None],
    ["read", "unverified", None, None],
]
UNSAFE_VERDICTS: Final[JsonValue] = [
    *(["ValueError"] * 4),
    *(["ok"] * 4),
    *(["ValueError"] * 5),
    "ok",
    "ValueError",
    "ValueError",
    "ok",
    "ok",
]
SECRET_EXPECTED: Final[dict[str, JsonValue]] = {
    "base_abstract": [
        "async_delete_secret",
        "async_read_secret",
        "async_write_secret",
        "sync_read_secret",
    ],
    "custom_abstract": ["async_read_secret", "sync_read_secret"],
    "half_custom": "TypeError",
    "half_base": "TypeError",
    "is_base": True,
    "kms_custom": "custom",
    "sync_read": ["1", None],
    "async_read": ["1", None],
    "write": {"name": "b", "status": "written"},
    "delete": {"name": "b", "deleted": True},
    "store_calls": STORE_CALLS,
    "store_data": {"keep": "k", "a2": "new"},
    "rotated": ["ok", {"name": "a2", "status": "written"}],
    "rotated_levels": [],
    "rotate_missing": ["ValueError", NOT_FOUND],
    "missing_levels": ERROR_LOGGED,
    "rotate_unverified": ["ValueError", "Failed to verify new secret unverified"],
    "unverified_levels": ERROR_LOGGED,
    "rotate_no_write": NO_WRITE,
    "no_write_levels": ERROR_LOGGED,
    "default_write": NO_WRITE,
    "default_delete": [
        "NotImplementedError",
        (
            "Delete operations are not implemented for custom_secret_manager. "
            "Override async_delete_secret() to add delete support."
        ),
    ],
    "init_records": [["INFO", False], ["INFO", False]],
    "names": ["custom_secret_manager", "vault"],
    "reprs": ["<ReadOnly(name=custom_secret_manager)>", "<ReadOnly(name=vault)>"],
    "validate": True,
    "health": [True, True],
    "unsafe": UNSAFE_VERDICTS,
    "unsafe_message": ["ValueError", "Invalid secret_name '../x'"],
}
SECRET_DEVIATIONS: Final[dict[str, tuple[JsonValue, JsonValue]]] = {
    "httpx_timeout": (["ValueError", "Timeout error occurred"], ["TimeoutException", "slow vault"]),
    "timeout_levels": ([], ERROR_LOGGED),
    "httpx_status": (
        ["ValueError", "HTTP error occurred: vault down"],
        ["HTTPStatusError", "bad status"],
    ),
    "status_levels": (ERROR_LOGGED, ERROR_LOGGED),
    "missing_messages": ([f"{ROTATE_LOG}: {NOT_FOUND}"], [ROTATE_LOG]),
}


@pytest.fixture(scope="module")
def reports() -> Reports:
    command = [sys.executable, "-c", PROBE]
    completed = subprocess.run(command, capture_output=True, text=True, check=False)
    assert completed.returncode == 0, completed.stderr
    return REPORT.validate_json(completed.stdout)


def test_prompt_matches_litellm(reports: Reports) -> None:
    theirs = reports["prompt"]["litellm"]
    ours = reports["prompt"]["betterlite"]
    assert sorted(ours) == sorted(theirs)
    for key in sorted(theirs):
        assert ours[key] == theirs[key], key


@pytest.mark.parametrize(("key", "expected"), list(PROMPT_EXPECTED.items()))
def test_prompt_expected(reports: Reports, key: str, expected: JsonValue) -> None:
    sides = reports["prompt"]
    assert sides["betterlite"][key] == expected
    assert sides["litellm"][key] == expected


def test_secret_matches_litellm(reports: Reports) -> None:
    theirs = reports["secret"]["litellm"]
    ours = reports["secret"]["betterlite"]
    assert sorted(ours) == sorted(theirs)
    for key in sorted(set(theirs) - set(SECRET_DEVIATIONS)):
        assert ours[key] == theirs[key], key


@pytest.mark.parametrize(("key", "expected"), list(SECRET_EXPECTED.items()))
def test_secret_expected(reports: Reports, key: str, expected: JsonValue) -> None:
    sides = reports["secret"]
    assert sides["betterlite"][key] == expected
    assert sides["litellm"][key] == expected


@pytest.mark.parametrize(("key", "pair"), list(SECRET_DEVIATIONS.items()))
def test_secret_deviation(reports: Reports, key: str, pair: tuple[JsonValue, JsonValue]) -> None:
    sides = reports["secret"]
    assert sides["litellm"][key] == pair[0]
    assert sides["betterlite"][key] == pair[1]
