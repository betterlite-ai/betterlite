import datetime as dt

from litellm.litellm_core_utils.rules import Rules

from b5367.flags import run_credentials, run_request_flags
from b5367.markers import CallbackValue, Json, Markers
from b5367.views import (
    LOGGING,
    STATE,
    UTILS,
    FixedRule,
    Input,
    ListsSpec,
    LoggingClassGetter,
    Recorder,
    StubLogging,
)


def entries_out(markers: Markers, values: list[CallbackValue] | None) -> Json:
    if values is None:
        return None
    return [markers.entry_out(value) for value in values]


def logging_out(markers: Markers, logging_obj: StubLogging) -> Json:
    fields = logging_obj.fields
    guardrails = fields["applied_guardrails"]
    return {
        "model": fields["model"],
        "call_type": fields["call_type"],
        "function_id": fields["function_id"],
        "stream": fields["stream"],
        "applied_guardrails": None if guardrails is None else [*guardrails],
        "dynamic": {
            "input": None,
            "success": entries_out(markers, fields["dynamic_success_callbacks"]),
            "async_success": entries_out(markers, fields["dynamic_async_success_callbacks"]),
            "failure": entries_out(markers, fields["dynamic_failure_callbacks"]),
            "async_failure": entries_out(markers, fields["dynamic_async_failure_callbacks"]),
        },
    }


def install_state(input_value: Input, markers: Markers) -> None:
    lists = input_value.get("lists")
    if lists is None:
        lists = ListsSpec()
    rules = input_value.get("rules")
    provider_map = input_value.get("provider_map")
    providers = input_value.get("providers")
    custom_providers = input_value.get("custom_providers")
    STATE.callbacks = markers.entries(input_value.get("callbacks"))
    STATE.input_callback = markers.entries(lists.get("input"))
    STATE.success_callback = markers.entries(lists.get("success"))
    STATE._async_success_callback = markers.entries(lists.get("async_success"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L200-L202
    STATE.failure_callback = markers.entries(lists.get("failure"))
    STATE._async_failure_callback = markers.entries(lists.get("async_failure"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L203-L205
    STATE.pre_call_rules = (
        [] if rules is None else [None if r is None else FixedRule(r) for r in rules]
    )
    STATE.custom_provider_map = (
        [] if provider_map is None else [{"provider": n} for n in provider_map]
    )
    STATE.provider_list = [] if providers is None else providers
    STATE._custom_providers = [] if custom_providers is None else custom_providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1528
    LOGGING._init_custom_logger_compatible_class = Recorder(input_value.get("resolver"), markers)  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L4502
    UTILS.get_litellm_logging_class = LoggingClassGetter()
    UTILS.callback_list = ["held by the case"]


def lists_out(markers: Markers) -> Json:
    return {
        "input": [markers.entry_out(item) for item in STATE.input_callback],
        "success": [markers.entry_out(item) for item in STATE.success_callback],
        "async_success": [markers.entry_out(item) for item in STATE._async_success_callback],  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L200-L202
        "failure": [markers.entry_out(item) for item in STATE.failure_callback],
        "async_failure": [markers.entry_out(item) for item in STATE._async_failure_callback],  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L203-L205
    }


def run_setup(input_value: Input) -> Json:
    markers = Markers()
    install_state(input_value, markers)
    async_flag = input_value.get("is_async_call")
    logging_obj, changed = UTILS.function_setup(
        original_function=input_value["call_type"],
        rules_obj=Rules(),
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        is_async_call=True if async_flag is None else async_flag,
        **input_value["request"],
    )
    return {
        "request": changed,
        "lists": lists_out(markers),
        "providers": {
            "providers": [*STATE.provider_list],
            "custom_providers": [*STATE._custom_providers],  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1528
        },
        "logging": logging_out(markers, logging_obj),
    }


def run(input_value: Input) -> Json:
    match input_value["action"]:
        case "setup":
            return run_setup(input_value)
        case "credentials":
            return run_credentials(input_value)
        case "request_flags":
            return run_request_flags(input_value)
