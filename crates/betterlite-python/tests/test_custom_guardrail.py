import subprocess
import sys

PROBE = """
import asyncio
import inspect
import json
import sys

import betterlite.exceptions
import litellm.exceptions
from betterlite.integrations.custom_guardrail import (
    DEFAULT_ADVISORY_MESSAGE as BETTER_ADVISORY,
    CustomGuardrail as BetterGuardrail,
    get_session_id_from_request_data as better_session_id,
    is_guardrail_intervention as better_intervention,
    log_guardrail_information as better_logguard,
)
from betterlite.types.guardrails.params import (
    GuardrailEventHooks as BetterHooks,
    LitellmParams as BetterParams,
    Mode as BetterMode,
)
from litellm.integrations.custom_guardrail import (
    DEFAULT_ADVISORY_MESSAGE as LITE_ADVISORY,
    CustomGuardrail as LiteGuardrail,
    get_session_id_from_request_data as lite_session_id,
    is_guardrail_intervention as lite_intervention,
    log_guardrail_information as lite_logguard,
)
from litellm.types.guardrails import (
    GuardrailEventHooks as LiteHooks,
    LitellmParams as LiteParams,
    Mode as LiteMode,
)

SIDES = {
    "litellm": {
        "base": LiteGuardrail,
        "hooks": LiteHooks,
        "params": LiteParams,
        "mode": LiteMode,
        "advisory": LITE_ADVISORY,
        "session": lite_session_id,
        "intervention": lite_intervention,
        "logguard": lite_logguard,
        "exceptions": litellm.exceptions,
    },
    "betterlite": {
        "base": BetterGuardrail,
        "hooks": BetterHooks,
        "params": BetterParams,
        "mode": BetterMode,
        "advisory": BETTER_ADVISORY,
        "session": better_session_id,
        "intervention": better_intervention,
        "logguard": better_logguard,
        "exceptions": betterlite.exceptions,
    },
}

HOOKS = ("pre_call", "post_call", "during_call", "logging_only")


def mismatch(label, left, right):
    print("MISMATCH " + label, file=sys.stderr)
    print("litellm: " + json.dumps(left, indent=2, default=str, sort_keys=True), file=sys.stderr)
    print(
        "betterlite: " + json.dumps(right, indent=2, default=str, sort_keys=True), file=sys.stderr
    )
    raise SystemExit(1)


def equal(label, left, right):
    if left != right:
        mismatch(label, left, right)


def param_rows(member):
    rows = []
    for parameter in inspect.signature(member).parameters.values():
        rows.append({"name": parameter.name, "default": repr(parameter.default)})
    return rows


def defined_methods(base):
    rows = {}
    for name, stored in base.__dict__.items():
        if isinstance(stored, staticmethod) or isinstance(stored, classmethod):
            member = stored.__func__
        elif inspect.isfunction(stored):
            member = stored
        else:
            continue
        rows[name] = {
            "is_async": inspect.iscoroutinefunction(member),
            "params": param_rows(member),
        }
    return rows


def all_methods(base):
    rows = defined_methods(base)
    for name in dir(base):
        if name in rows:
            continue
        stored = inspect.getattr_static(base, name)
        if isinstance(stored, staticmethod) or isinstance(stored, classmethod):
            member = stored.__func__
        elif inspect.isfunction(stored):
            member = stored
        else:
            continue
        rows[name] = {
            "is_async": inspect.iscoroutinefunction(member),
            "params": param_rows(member),
        }
    return rows


def class_attributes(base):
    names = []
    for name in base.__dict__:
        if name.startswith("__"):
            continue
        names.append(name)
    return sorted(names)


def group_surface():
    theirs = defined_methods(LiteGuardrail)
    ours = all_methods(BetterGuardrail)
    for name, row in sorted(theirs.items()):
        if name not in ours:
            mismatch("missing method " + name, sorted(ours), sorted(theirs))
        equal("method " + name + " async", row["is_async"], ours[name]["is_async"])
        our_params = {item["name"]: item["default"] for item in ours[name]["params"]}
        for parameter in row["params"]:
            if parameter["name"] not in our_params:
                mismatch(
                    "param " + name + "." + parameter["name"], sorted(our_params), row["params"]
                )
            equal(
                "param default " + name + "." + parameter["name"],
                parameter["default"],
                our_params[parameter["name"]],
            )
    for name in class_attributes(LiteGuardrail):
        if not hasattr(BetterGuardrail, name):
            mismatch(
                "missing attribute " + name,
                sorted(class_attributes(BetterGuardrail)),
                sorted(class_attributes(LiteGuardrail)),
            )
    equal("advisory constant", LITE_ADVISORY, BETTER_ADVISORY)
    for label in ("session", "intervention", "logguard"):
        equal(label + " async", False, inspect.iscoroutinefunction(SIDES["betterlite"][label]))
        equal(
            label + " params",
            [item["name"] for item in param_rows(SIDES["litellm"][label])],
            [item["name"] for item in param_rows(SIDES["betterlite"][label])],
        )

    bodies = {}
    for side, types in SIDES.items():
        hooks = types["hooks"]
        base = types["base"]

        class ProbeGuardrail(base):
            def __init__(self):
                super().__init__(guardrail_name="probe", supported_event_hooks=[hooks.pre_call])

            async def apply_guardrail(self, inputs, request_data, input_type, logging_obj=None):
                return inputs

        body = {}
        body["constructs"] = isinstance(ProbeGuardrail(), base)
        stored = ProbeGuardrail.__dict__["apply_guardrail"]
        body["marker"] = bool(getattr(stored, "_litellm_logs_guardrail_information", False))
        body["replaced"] = stored is not base.apply_guardrail
        body["wrapper_async"] = inspect.iscoroutinefunction(ProbeGuardrail.apply_guardrail)
        body["hook_runs"] = asyncio.run(
            ProbeGuardrail().apply_guardrail(
                inputs={"texts": ["hi"]}, request_data={}, input_type="request"
            )
        )
        bodies[side] = body
    equal("subclass body", bodies["litellm"], bodies["betterlite"])
    print("OK")


def answer(instance, data, hooks, event):
    try:
        return ["ok", instance.should_run_guardrail(data, hooks[event])]
    except Exception as error:
        return ["raise", type(error).__name__]


def group_selection():
    reports = {}
    for side, types in SIDES.items():
        hooks = types["hooks"]
        base = types["base"]
        mode = types["mode"]
        report = {}
        instances = {
            "plain": base(guardrail_name="probe"),
            "pre": base(guardrail_name="probe", event_hook=hooks.pre_call),
            "string": base(guardrail_name="probe", event_hook="pre_call"),
            "multi": base(guardrail_name="probe", event_hook=[hooks.pre_call, hooks.post_call]),
            "tagged": base(
                guardrail_name="probe",
                event_hook=mode(
                    tags={"team": [hooks.pre_call, hooks.post_call]}, default=hooks.post_call
                ),
            ),
            "on": base(guardrail_name="probe", default_on=True),
            "off": base(guardrail_name="probe", default_on=False),
        }
        datasets = {
            "empty": {},
            "named": {"guardrails": ["probe"]},
            "other": {"guardrails": ["other"]},
            "metadata": {"metadata": {"guardrails": ["probe"]}},
            "thread": {"litellm_metadata": {"guardrails": ["probe"]}},
            "tag": {"metadata": {"tags": ["team"]}},
            "logged": {"guardrails": ["probe"], "litellm_logging_obj": object()},
        }
        for instance_name, instance in instances.items():
            for data_name, data in datasets.items():
                for event in HOOKS:
                    key = "|".join((instance_name, data_name, event))
                    report[key] = answer(instance, data, hooks, event)
        for event in HOOKS:
            plain_hook = instances["plain"]
            report["event|plain|" + event] = plain_hook._event_hook_is_event_type(hooks[event])
            report["event|pre|" + event] = instances["pre"]._event_hook_is_event_type(hooks[event])
        if side == "betterlite":
            suppressed = instances["pre"].should_run_guardrail(
                {"guardrails": ["probe"]}, hooks.pre_call, suppressed=frozenset({"probe"})
            )
            report["suppressed"] = suppressed
        reports[side] = report
    ours = reports["betterlite"]
    if ours.pop("suppressed") is not False:
        mismatch("suppressed guardrail ran", False, True)
    equal("selection", reports["litellm"], ours)
    print("OK")


def capture(call):
    try:
        call()
    except Exception as error:
        return type(error).__name__
    return "none"


def group_errors():
    reports = {}
    for side, types in SIDES.items():
        hooks = types["hooks"]
        base = types["base"]
        mode = types["mode"]
        report = {
            "bad_hook": capture(
                lambda: base(
                    guardrail_name="x",
                    supported_event_hooks=[hooks.pre_call],
                    event_hook=hooks.post_call,
                )
            ),
            "unknown_hook": capture(
                lambda: base(
                    guardrail_name="x",
                    supported_event_hooks=[hooks.pre_call],
                    event_hook="not_a_hook",
                )
            ),
            "tag_mode": capture(
                lambda: base(
                    guardrail_name="x",
                    supported_event_hooks=[hooks.pre_call],
                    event_hook=mode(tags={"team": hooks.post_call}),
                )
            ),
            "non_list": capture(
                lambda: base(
                    guardrail_name="probe", event_hook=hooks.pre_call
                ).should_run_guardrail({"guardrails": 5}, hooks.pre_call)
            ),
        }
        guardrail = base(guardrail_name="probe", event_hook=hooks.pre_call)
        raised = types["exceptions"].GuardrailRaisedException(message="blocked")
        denied = types["exceptions"].BlockedPiiEntityError("EMAIL")
        report["intervention"] = [
            types["intervention"](raised),
            types["intervention"](denied),
            types["intervention"](ValueError("plain")),
        ]
        data = {"guardrails": ["probe"]}
        try:
            guardrail._process_error(raised, data, 1.0, 2.0)
        except Exception:
            pass
        report["status"] = normalize(data["metadata"])
        reports[side] = report
    equal("errors", reports["litellm"], reports["betterlite"])
    print("OK")


def advisory(guardrail, data):
    written = guardrail.inject_advisory_message(data, "Advisory note.")
    return [written, data]


def group_messages():
    reports = {}
    for side, types in SIDES.items():
        hooks = types["hooks"]
        base = types["base"]
        params = types["params"]
        guardrail = base(guardrail_name="probe", event_hook=hooks.pre_call)
        templated = base(
            guardrail_name="probe",
            violation_message_template="Blocked: {default_message} ({reason})",
        )
        report = {
            "default": guardrail.render_violation_message("fallback message"),
            "template": templated.render_violation_message("fallback", {"reason": "pii"}),
            "number": base(
                guardrail_name="probe", violation_message_template="{count} hits"
            ).render_violation_message("fallback", {"count": 3}),
            "missing": base(
                guardrail_name="probe", violation_message_template="{missing_key}"
            ).render_violation_message("fallback"),
            "advisory_messages": advisory(
                guardrail, {"messages": [{"role": "user", "content": "hi"}]}
            ),
            "advisory_instructions": advisory(guardrail, {"instructions": "be careful"}),
            "advisory_both": advisory(
                guardrail, {"instructions": "be careful", "messages": []}
            ),
            "advisory_input": advisory(guardrail, {"input": "hello"}),
            "advisory_structured": advisory(
                guardrail, {"input": [{"role": "user", "content": "hi"}], "messages": []}
            ),
            "advisory_empty": advisory(guardrail, {}),
            "masked": guardrail.mask_content_in_string("abcdef", "**", 2, 4),
            "masked_outside": guardrail.mask_content_in_string("abc", "**", 2, 9),
            "config_model": str(guardrail.get_config_model()),
            "supported_hooks": str(guardrail.get_supported_event_hooks()),
            "routes": guardrail.should_route_on_sensitive_data(),
        }
        updated = base(guardrail_name="probe")
        updated.update_in_memory_litellm_params(
            params(
                guardrail="probe",
                mode="pre_call",
                default_on=True,
                mask_request_content=True,
                violation_message_template="Updated {default_message}",
                timeout=2.5,
            )
        )
        report["updated"] = {
            name: str(getattr(updated, name, "<missing>"))
            for name in (
                "default_on",
                "mask_request_content",
                "mask_response_content",
                "violation_message_template",
                "timeout",
                "guardrail",
                "mode",
                "sticky_session_routing",
                "only_scan_new_messages",
            )
        }
        reports[side] = report
    equal("messages", reports["litellm"], reports["betterlite"])
    print("OK")


class FakeLogging:
    def __init__(self):
        self.litellm_params = {"metadata": {}}
        self.model_call_details = {"litellm_params": {"metadata": {}}}


def normalize(container):
    entries = container.get("standard_logging_guardrail_information")
    if entries is None:
        return None
    normalized = []
    for entry in entries:
        if hasattr(entry, "model_dump"):
            entry = entry.model_dump(mode="json")
        kept = {
            key: value
            for key, value in entry.items()
            if key not in ("start_time", "end_time", "duration") and value is not None
        }
        normalized.append(kept)
    return normalized


def group_records():
    reports = {}

    async def run(side, types):
        hooks = types["hooks"]
        base = types["base"]
        guardrail = base(guardrail_name="probe", event_hook=hooks.pre_call)
        data = {
            "guardrails": ["probe"],
            "metadata": {"user": "u"},
            "litellm_logging_obj": FakeLogging(),
        }
        guardrail.add_standard_logging_guardrail_information_to_request_data(
            {"action": "allow"},
            data,
            "success",
            start_time=100.0,
            end_time=101.0,
            duration=1.0,
            event_type=hooks.pre_call,
        )
        logging_obj = data["litellm_logging_obj"]
        direct = {
            "metadata": normalize(data["metadata"]),
            "params": normalize(logging_obj.litellm_params["metadata"]),
            "details": normalize(logging_obj.model_call_details["litellm_params"]["metadata"]),
        }

        class ProbeGuardrail(base):
            def __init__(self):
                super().__init__(guardrail_name="probe", event_hook=hooks.pre_call)

            async def apply_guardrail(self, inputs, request_data, input_type, logging_obj=None):
                if inputs.get("texts") == ["boom"]:
                    raise ValueError("boom")
                return inputs

        probe = ProbeGuardrail()
        success_logging = FakeLogging()
        result = await probe.apply_guardrail(
            inputs={"texts": ["hi"]},
            request_data={"guardrails": ["probe"]},
            input_type="request",
            logging_obj=success_logging,
        )
        failure_logging = FakeLogging()
        try:
            await probe.apply_guardrail(
                inputs={"texts": ["boom"]},
                request_data={"guardrails": ["probe"]},
                input_type="request",
                logging_obj=failure_logging,
            )
            raised = "none"
        except Exception as error:
            raised = type(error).__name__

        class CustomCodeGuardrailProbe(base):
            def __init__(self):
                super().__init__(guardrail_name="probe", event_hook=hooks.pre_call)

            async def apply_guardrail(self, inputs, request_data, input_type, logging_obj=None):
                raise ValueError("denied")

        deny_logging = FakeLogging()
        try:
            await CustomCodeGuardrailProbe().apply_guardrail(
                inputs={"texts": ["hi"]},
                request_data={"guardrails": ["probe"]},
                input_type="request",
                logging_obj=deny_logging,
            )
        except Exception:
            pass

        updated = base(guardrail_name="probe", event_hook=hooks.pre_call)
        updated.update_in_memory_litellm_params(
            types["params"](guardrail="probe", mode="pre_call")
        )
        updated_logging = FakeLogging()
        updated.add_standard_logging_guardrail_information_to_request_data(
            {"action": "allow"},
            {"guardrails": ["probe"], "litellm_logging_obj": updated_logging},
            "success",
            event_type=hooks.pre_call,
        )
        written = normalize(updated_logging.litellm_params["metadata"]) or []

        class BrokenCache:
            async def async_get_cache(self, key):
                raise RuntimeError("cache down")

            async def async_set_cache(self, key, value, ttl):
                raise RuntimeError("cache down")

        scanning = base(
            guardrail_name="probe", event_hook=hooks.pre_call, only_scan_new_messages=True
        )
        scanning_data = {"metadata": {"session_id": "s-1"}}
        filtered = await scanning.filter_new_texts_for_session(
            ["hello"], scanning_data, BrokenCache()
        )
        await scanning.mark_texts_scanned(["hello"], scanning_data, BrokenCache())
        return {
            "direct": direct,
            "result": result,
            "raised": raised,
            "success": normalize(success_logging.litellm_params["metadata"]),
            "failure": normalize(failure_logging.litellm_params["metadata"]),
            "deny": normalize(deny_logging.litellm_params["metadata"]),
            "updated": [entry["guardrail_status"] for entry in written],
            "filtered": filtered,
        }

    for side, types in SIDES.items():
        reports[side] = asyncio.run(run(side, types))
    equal("records", reports["litellm"], reports["betterlite"])
    print("OK")


GROUPS = {
    "surface": group_surface,
    "selection": group_selection,
    "errors": group_errors,
    "messages": group_messages,
    "records": group_records,
}

GROUPS[sys.argv[1]]()
"""


def run_probe(group: str) -> str:
    completed = subprocess.run(
        [sys.executable, "-c", PROBE, group], capture_output=True, text=True, check=False
    )
    assert completed.returncode == 0, completed.stderr
    return completed.stdout.strip()


def test_guardrail_surface() -> None:
    output = run_probe("surface")
    assert output == "OK"


def test_guardrail_selection() -> None:
    output = run_probe("selection")
    assert output == "OK"


def test_guardrail_errors() -> None:
    output = run_probe("errors")
    assert output == "OK"


def test_guardrail_messages() -> None:
    output = run_probe("messages")
    assert output == "OK"


def test_guardrail_records() -> None:
    output = run_probe("records")
    assert output == "OK"
