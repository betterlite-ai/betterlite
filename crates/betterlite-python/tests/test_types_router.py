import json
import subprocess
import sys
from collections.abc import Sequence
from typing import Final

from pydantic import JsonValue, TypeAdapter

LITELLM_PREFIX: Final = "litellm.types"
LITELLM_MODULE: Final = "router"
OURS_PREFIX: Final = "betterlite"
OURS_MODULE: Final = "_router_types"
EXCLUDED: Final = {"CustomRoutingStrategyBase"}
PROTOCOLS: Final = (
    "FallbackAccessCheck", "FallbackBudgetCheck", "AutoRouterCapabilityLimit",
    "PreRoutingStrategy", "RoutingPlugin", "ClassifierPlugin",
)
GENERATED_ID_LENGTH: Final = 36
MOCK_INT_MARKER: Final = 1
EXTRA_MODELS: Final = 19
FORBIDDEN_HEADERS: Final = (
    "api-key", "authorization", "connection", "content-length", "content-type", "cookie",
    "host", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection",
    "set-cookie", "te", "trailer", "transfer-encoding", "upgrade", "www-authenticate",
    "x-api-key", "x-goog-api-key",
)
FORBIDDEN_HEADER_MESSAGE: Final = (
    "Value error, provider_affinity_header cannot be an authentication, cookie, "
    "or transport header"
)
HEADER_NAME_MESSAGE: Final = (
    "Value error, provider_affinity_header must be a valid HTTP header name"
)
SIBLING_TYPES: Final[tuple[str, ...]] = (
    "SearchProviders",
    "StandardLoggingRoutingDecision",
    "MirroredPricingParams",
    "CustomPricingLiteLLMParams",
)

PROBE: Final = """
import ast, dataclasses, datetime, enum, httpx, importlib, inspect, json, sys, typing
from pydantic import BaseModel, TypeAdapter, ValidationError
from pydantic_core import to_jsonable_python
from typing_extensions import ReadOnly

prefix, name, mode, spec = sys.argv[1], sys.argv[2], sys.argv[3], json.loads(sys.argv[4])


def load():
    return importlib.import_module(prefix + "." + name)


def literals(tp):
    if typing.get_origin(tp) is typing.Literal:
        return [list(typing.get_args(tp))]
    found = []
    for arg in typing.get_args(tp):
        found.extend(literals(arg))
    return found


def named(value):
    return value.__name__ if inspect.isclass(value) else repr(value)


def plain(value):
    return to_jsonable_python(value, fallback=named)


def default_of(field):
    if field.default_factory is not None:
        return {"factory": plain(field.default_factory())}
    return {"value": plain(field.default)}


def describe(obj):
    if inspect.isclass(obj) and issubclass(obj, BaseModel):
        fields = []
        for field_name, field in obj.model_fields.items():
            entry = {
                "name": field_name,
                "required": field.is_required(),
                "literals": literals(field.annotation),
            }
            if not field.is_required():
                entry["default"] = default_of(field)
            fields.append(entry)
        return {"kind": "model", "fields": fields}
    if inspect.isclass(obj) and issubclass(obj, enum.Enum):
        members = [
            [member_name, plain(member.value)] for member_name, member in obj.__members__.items()
        ]
        return {"kind": "enum", "members": members}
    if inspect.isclass(obj) and hasattr(obj, "__required_keys__"):
        annotations = obj.__annotations__
        return {
            "kind": "typeddict",
            "total": obj.__total__,
            "keys": list(annotations),
            "required": sorted(obj.__required_keys__),
            "optional": sorted(obj.__optional_keys__),
            "readonly": sorted(
                key
                for key, annotation in annotations.items()
                if typing.get_origin(annotation) is ReadOnly
            ),
            "literals": [[key, literals(annotation)] for key, annotation in annotations.items()],
        }
    if inspect.isclass(obj) and dataclasses.is_dataclass(obj):
        fields = []
        for item in dataclasses.fields(obj):
            fields.append(
                {
                    "name": item.name,
                    "required": item.default is dataclasses.MISSING
                    and item.default_factory is dataclasses.MISSING,
                    "default": None if item.default is dataclasses.MISSING else plain(item.default),
                    "kw_only": item.kw_only,
                }
            )
        return {
            "kind": "dataclass",
            "frozen": obj.__dataclass_params__.frozen,
            "slots": hasattr(obj, "__slots__"),
            "fields": fields,
        }
    if inspect.isclass(obj) and getattr(obj, "_is_protocol", False):
        methods = inspect.getmembers(obj, predicate=inspect.isfunction)
        return {"kind": "protocol", "methods": sorted(method for method, _ in methods)}
    if typing.get_origin(obj) is not None:
        return {"kind": "alias", "schema": TypeAdapter(obj).json_schema()}
    return {"kind": "value", "value": plain(obj)}


def public_names(module):
    tree = ast.parse(open(module.__file__).read())
    found = []
    for node in tree.body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            found.append(node.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    found.append(target.id)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            found.append(node.target.id)
    return sorted({item for item in found if not item.startswith("_")})


def signatures(names):
    module = load()
    result = {}
    for item_name in names:
        item = getattr(module, item_name)
        methods = dict(inspect.getmembers(item, predicate=inspect.isfunction))
        if inspect.isclass(item) and inspect.isfunction(getattr(item, "__init__", None)):
            methods.setdefault("__init__", item.__init__)
        for method_name, member in methods.items():
            result[item_name + "." + method_name] = [
                [parameter.name, parameter.kind.name, plain(parameter.default)]
                for parameter in inspect.signature(member).parameters.values()
            ]
    return result


def outcome(call):
    try:
        value = call()
    except ValidationError as error:
        errors = [[item["type"], list(item["loc"]), item["msg"]] for item in error.errors()]
        return {"validation": errors}
    except ValueError as error:
        return {"error": [type(error).__name__, str(error)]}
    except (AttributeError, KeyError, TypeError) as error:
        return {"raised": type(error).__name__}
    return {"ok": plain(value)}


def accessors(value, key):
    result = {
        "getitem": outcome(lambda: value[key]),
        "contains": outcome(lambda: key in value),
        "contains_missing": outcome(lambda: "nope" in value),
        "get_missing": outcome(lambda: value.get("nope")),
        "get_default": outcome(lambda: value.get("nope", "d")),
        "missing": outcome(lambda: value["nope"]),
    }
    value["custom"] = 7
    result["custom"] = outcome(lambda: value.custom)
    result["contains_custom"] = outcome(lambda: "custom" in value)
    result["dump_custom"] = outcome(lambda: value.model_dump()["custom"])
    return result


def model_info_results():
    module = load()
    result = {}
    generated = module.ModelInfo()
    result["generated_id"] = {"str": isinstance(generated.id, str), "length": len(generated.id)}
    result["int_id"] = outcome(lambda: module.ModelInfo(id=5).id)
    result["str_id"] = outcome(lambda: module.ModelInfo(id="abc").id)
    result["bool_id"] = outcome(lambda: module.ModelInfo(id=True).id)
    result["float_id"] = outcome(lambda: module.ModelInfo(id=1.5))
    result["dump"] = outcome(
        lambda: module.ModelInfo(id="x", base_model="b", ptu_count=2).model_dump()
    )
    result["extra"] = outcome(lambda: module.ModelInfo(id="x", custom_field=1).model_dump())
    for value in (0, -1, 1, 1000000, 1000001):
        result["ptu_count_" + str(value)] = outcome(
            lambda: module.ModelInfo(id="c", ptu_count=value).model_dump()
        )
    for value in (-0.1, 0.0, 1000000.0, 1000000.5):
        result["ptu_cost_" + str(value)] = outcome(
            lambda: module.ModelInfo(id="c", cost_per_ptu_per_hour=value).model_dump()
        )
    naive = datetime.datetime(2026, 1, 1)
    aware = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
    result["window"] = outcome(
        lambda: module.ModelInfo(
            id="w", ptu_effective_from=naive, ptu_effective_to=aware + datetime.timedelta(days=1)
        ).model_dump()
    )
    result["window_equal"] = outcome(
        lambda: module.ModelInfo(ptu_effective_from=naive, ptu_effective_to=naive)
    )
    result["window_backwards"] = outcome(
        lambda: module.ModelInfo(ptu_effective_from=aware, ptu_effective_to=naive)
    )
    result["window_mixed"] = outcome(
        lambda: module.ModelInfo(
            ptu_effective_from=naive,
            ptu_effective_to=datetime.datetime(2025, 12, 31, 23, 0, tzinfo=datetime.timezone.utc),
        )
    )
    return result


def params_results():
    module = load()
    result = {}
    reserved = {"self": "drop", "params": {"a": 1}, "__class__": "x", "max_retries": "4"}
    reserved_instance = module.GenericLiteLLMParams(**reserved)
    result["reserved"] = {
        "max_retries": plain(reserved_instance.max_retries),
        "extra": plain(reserved_instance.__pydantic_extra__),
    }
    result["max_retries_str"] = outcome(
        lambda: module.GenericLiteLLMParams(max_retries="4").max_retries
    )
    result["max_retries_bad"] = outcome(lambda: module.GenericLiteLLMParams(max_retries="four"))
    drop_values = [None, True, False, "true", " FALSE ", "yes", 1, 0, 2, "bogus", ["x"]]
    for index, value in enumerate(drop_values):
        dropped = module.GenericLiteLLMParams(drop_params=value).drop_params
        result["drop_" + str(index)] = {"value": plain(dropped), "type": type(dropped).__name__}
    headers = [
        "api-key", "authorization", "connection", "content-length", "content-type", "cookie",
        "host", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection",
        "set-cookie", "te", "trailer", "transfer-encoding", "upgrade", "www-authenticate",
        "x-api-key", "x-goog-api-key", "X-Session", "X-Api-Key", "bad header", "",
    ]
    result["headers"] = {
        header: outcome(
            lambda: module.GenericLiteLLMParams(
                provider_affinity_header=header
            ).provider_affinity_header
        )
        for header in headers
    }
    result["header_none"] = outcome(
        lambda: module.GenericLiteLLMParams(provider_affinity_header=None).provider_affinity_header
    )
    result["params_requires_model"] = outcome(lambda: module.LiteLLM_Params(api_key="k"))
    result["params_dump"] = outcome(
        lambda: module.LiteLLM_Params(model="m", api_key="k").model_dump()
    )
    result["update_dump"] = outcome(lambda: module.updateLiteLLMParams(api_key="k").model_dump())
    result["access"] = accessors(module.GenericLiteLLMParams(api_key="k"), "api_key")
    return result


def deployment_results():
    module = load()
    result = {}
    params = module.LiteLLM_Params(model="m", input_cost_per_token=0.5, output_cost_per_token=0.25)
    instance = module.Deployment(
        model_name="g", litellm_params=params, model_info=module.ModelInfo(id="x", base_model="b")
    )
    result["mirror"] = {
        "input": plain(instance.model_info.input_cost_per_token),
        "output": plain(instance.model_info.output_cost_per_token),
    }
    result["dump"] = outcome(lambda: instance.to_json())
    result["exclude_none"] = outcome(lambda: instance.to_json(exclude_none=True))
    dict_params = module.Deployment(
        model_name="h",
        litellm_params={"model": "m", "input_cost_per_token": 0.25},
        model_info={"id": "y", "db_model": True},
    )
    result["dict_params_mirror"] = plain(dict_params.model_info.input_cost_per_token)
    result["dict_params_dump"] = outcome(lambda: dict_params.to_json())
    generated = module.Deployment(
        model_name="i",
        litellm_params=module.LiteLLM_Params(model="m"),
        model_info={"db_model": True},
    )
    result["dict_info_id"] = {
        "str": isinstance(generated.model_info.id, str),
        "length": len(generated.model_info.id),
    }
    generated_dump = generated.to_json()
    generated_dump["model_info"].pop("id")
    result["dict_info_dump"] = plain(generated_dump)
    bare = module.Deployment(model_name="n", litellm_params=module.LiteLLM_Params(model="m"))
    result["bare_id"] = {
        "str": isinstance(bare.model_info.id, str),
        "length": len(bare.model_info.id),
    }
    bare_dump = bare.to_json()
    bare_dump["model_info"].pop("id")
    result["bare_dump"] = plain(bare_dump)
    extra = module.Deployment(
        model_name="j",
        litellm_params=module.LiteLLM_Params(model="m"),
        model_info=module.ModelInfo(id="z"),
        custom=3,
    )
    result["extra_dump"] = outcome(lambda: extra.to_json())
    result["access"] = accessors(
        module.Deployment(
            model_name="k",
            litellm_params=module.LiteLLM_Params(model="m"),
            model_info=module.ModelInfo(id="a"),
        ),
        "model_name",
    )
    return result


def extra_results():
    module = load()
    cases = {
        "ModelConfig": {
            "model_name": "m", "litellm_params": {"model": "m", "messages": []},
            "tpm": 1, "rpm": 2,
        },
        "RoutingGroup": {
            "group_name": "g", "models": ["a"], "routing_strategy": "simple-shuffle",
        },
        "RouterConfig": {"model_list": []},
        "RetryPolicy": {},
        "UpdateRouterConfig": {},
        "RouterGeneralSettings": {},
        "GenericBudgetWindowDetails": {
            "budget_start": 1.0, "spend_key": "s", "start_time_key": "t", "ttl_seconds": 60,
        },
        "AllowedFailsPolicy": {},
        "AlertingConfig": {"webhook_url": "http://example.test"},
        "ModelGroupInfo": {"model_group": "g", "providers": ["p"]},
        "FineTuningConfig": {"custom_llm_provider": "openai"},
        "updateDeployment": {},
        "CredentialLiteLLMParams": {},
        "ModelGroupSettings": {},
        "PreRoutingHookResponse": {"model": "m", "messages": []},
        "RoutingContext": {
            "raw_messages": [], "structured_messages": [], "candidate_models": [],
        },
        "AdaptiveRouterWeights": {},
        "AdaptiveRouterConfig": {"available_models": []},
        "AdaptiveRouterPreferences": {"quality_tier": 1},
    }
    result = {}
    for name, payload in cases.items():
        instance = getattr(module, name)(**payload, unknown=1)
        dumped = plain(instance.model_dump())
        result[name] = {
            "has_unknown": hasattr(instance, "unknown"),
            "contains": "unknown" in dumped,
            "dump": dumped,
        }
    return result


def groups_results():
    module = load()
    result = {}
    result["default_dump"] = outcome(
        lambda: module.ModelGroupInfo(model_group="g", providers=["p"]).model_dump()
    )
    for index, value in enumerate([None, True, False, "yes", "no", 0, 1]):
        result["vision_" + str(index)] = outcome(
            lambda: module.ModelGroupInfo(
                model_group="g", providers=["p"], supports_vision=value
            ).supports_vision
        )
    for bool_name in (
        "supports_parallel_function_calling",
        "supports_vision",
        "supports_web_search",
        "supports_url_context",
        "supports_reasoning",
        "supports_function_calling",
        "supports_fast_mode",
    ):
        result["null_" + bool_name] = outcome(
            lambda: getattr(
                module.ModelGroupInfo(model_group="g", providers=["p"], **{bool_name: None}),
                bool_name,
            )
        )
    result["limits"] = outcome(
        lambda: module.ModelGroupInfo(
            model_group="g", providers=["p"], max_input_tokens=1.5, max_output_tokens=2
        ).model_dump()
    )
    first = module.ModelGroupInfo(model_group="g", providers=["p"])
    second = module.ModelGroupInfo(model_group="g", providers=["p"])
    first.supported_openai_params.append("temperature")
    result["shared_default"] = {
        "first": plain(first.supported_openai_params),
        "second": plain(second.supported_openai_params),
    }
    result["mode_valid"] = outcome(
        lambda: module.ModelGroupInfo(model_group="g", providers=["p"], mode="chat").mode
    )
    result["mode_other"] = outcome(
        lambda: module.ModelGroupInfo(model_group="g", providers=["p"], mode="bogus").mode
    )
    result["mode_invalid"] = outcome(
        lambda: module.ModelGroupInfo(model_group="g", providers=["p"], mode=5)
    )
    result["auth_valid"] = outcome(
        lambda: module.ModelGroupInfo(
            model_group="g",
            providers=["p"],
            configurable_clientside_auth_params=[{"api_base": "b"}],
        ).model_dump()["configurable_clientside_auth_params"]
    )
    result["auth_invalid"] = outcome(
        lambda: module.ModelGroupInfo(
            model_group="g",
            providers=["p"],
            configurable_clientside_auth_params=[1],
        )
    )
    return result


def errors_results():
    module = load()
    result = {}
    basic = module.RouterRateLimitErrorBasic(model="m")
    result["basic"] = {
        "message": str(basic),
        "model": plain(basic.model),
        "value_error": isinstance(basic, ValueError),
    }
    cases = [
        (12.5, True, ["a", "b"], ()),
        (12.5, False, ["a"], ("a",)),
        (1, False, ["a", "b"], ("a",)),
        (1, False, ["a", "b"], ("c",)),
        (0.0, True, [], ()),
    ]
    for index, (cooldown_time, pre_call, cooldown_list, model_ids) in enumerate(cases):
        error = module.RouterRateLimitError(
            model="m",
            cooldown_time=cooldown_time,
            enable_pre_call_checks=pre_call,
            cooldown_list=cooldown_list,
            model_ids=model_ids,
        )
        result["rate_" + str(index)] = {
            "message": str(error),
            "type": plain(error.type),
            "all": plain(error.all_deployments_in_cooldown),
            "cooldown_time": plain(error.cooldown_time),
            "cooldown_list": plain(error.cooldown_list),
            "model": plain(error.model),
            "value_error": isinstance(error, ValueError),
        }
    without_ids = module.RouterRateLimitError(
        model="n", cooldown_time=2, enable_pre_call_checks=True, cooldown_list=[]
    )
    result["default_ids"] = {
        "message": str(without_ids),
        "all": plain(without_ids.all_deployments_in_cooldown),
    }
    return result


def routing_group_results():
    module = load()
    result = {}
    result["valid"] = outcome(
        lambda: module.RoutingGroup(
            group_name="g",
            models=["a", "b"],
            routing_strategy="priority",
            model_priorities={"a": 1, "b": 2},
        ).model_dump()
    )
    cases = [
        ("simple-shuffle", ["a"], {"a": 1}, None),
        ("simple-shuffle", ["a"], None, {"x": 1}),
        ("priority", [], None, None),
        ("priority", ["a", "a"], None, None),
        ("priority", ["a", "b"], {"a": 1}, None),
        ("priority", ["a"], {"a": 1}, {"x": 1}),
        ("priority", ["a"], {"a": 1}, {}),
        ("priority", ["a"], {"a": 0}, None),
        ("priority", ["a"], {"a": 1.5}, None),
        ("priority", ["a"], {"a": True}, None),
        ("priority", ["a"], {"a": "1"}, None),
        ("priority", ["a"], {}, None),
        ("priority", ["a"], None, None),
        ("priority", ["a"], {"a": 1}, None),
    ]
    for index, (strategy, models, priorities, args) in enumerate(cases):
        result["case_" + str(index)] = outcome(
            lambda: module.RoutingGroup(
                group_name="g",
                models=models,
                routing_strategy=strategy,
                model_priorities=priorities,
                routing_strategy_args=args,
            ).model_dump()
        )
    return result


def access_window_results():
    module = load()
    result = {"valid": outcome(lambda: module.ModelAccessWindow(
        start="09:00", end="17:00", timezone="Europe/Berlin", team_ids=("t1",)
    ).model_dump())}
    cases = [
        {"start": "09:00+00:00", "end": "17:00", "timezone": "UTC", "team_ids": ("t1",)},
        {"start": "09:00", "end": "17:00", "timezone": "Mars/Olympus", "team_ids": ("t1",)},
        {"start": "09:00", "end": "17:00", "timezone": "UTC", "team_ids": ("t1", "")},
        {"start": "09:00", "end": "09:00", "timezone": "UTC", "team_ids": ("t1",)},
        {"start": "09:00", "end": "17:00", "timezone": "UTC", "team_ids": ()},
        {"start": "09:00", "end": "17:00", "timezone": "UTC", "team_ids": ("t1",), "extra": 1},
        {"start": "09:00", "end": "17:00", "timezone": "UTC"},
        {"start": "9am", "end": "5pm", "timezone": "UTC", "team_ids": ("t1",)},
    ]
    for index, case in enumerate(cases):
        result["case_" + str(index)] = outcome(lambda: module.ModelAccessWindow(**case))
    window = module.ModelAccessWindow(
        start="09:00", end="17:00", timezone="UTC", team_ids=("t1",)
    )
    result["frozen"] = outcome(lambda: setattr(window, "timezone", "Europe/Berlin"))
    return result


def adaptive_results():
    module = load()
    result = {"default": outcome(lambda: module.AdaptiveRouterWeights().model_dump())}
    weight_pairs = ((0.7, 0.3), (0.5, 0.5), (0.5, 0.5009), (1.0, 0.0), (0.0, 1.0), (0.8, 0.1))
    for quality, cost in weight_pairs:
        result["pair_" + str(quality) + "_" + str(cost)] = outcome(
            lambda: module.AdaptiveRouterWeights(quality=quality, cost=cost).model_dump()
        )
    result["quality_only"] = outcome(lambda: module.AdaptiveRouterWeights(quality=0.9).model_dump())
    result["cost_only"] = outcome(lambda: module.AdaptiveRouterWeights(cost=0.5))
    result["out_of_range"] = outcome(lambda: module.AdaptiveRouterWeights(quality=1.5, cost=-0.5))
    result["config"] = outcome(
        lambda: module.AdaptiveRouterConfig(available_models=["a"]).model_dump()
    )
    result["preferences"] = outcome(
        lambda: module.AdaptiveRouterPreferences(
            quality_tier=2, strengths=[module.RequestType.CODE_GENERATION]
        ).model_dump()
    )
    result["preferences_bad_tier"] = outcome(
        lambda: module.AdaptiveRouterPreferences(quality_tier=4)
    )
    return result


def mock_results():
    module = load()
    result = {}
    cases = [
        {"mock_testing_fallbacks": "TRUE", "mock_testing_context_fallbacks": False, "other": 1},
        {"mock_testing_fallbacks": "true"},
        {"mock_testing_fallbacks": " false "},
        {"mock_testing_fallbacks": "yes"},
        {"mock_testing_fallbacks": "bogus"},
        {"mock_testing_fallbacks": None},
        {"mock_testing_context_fallbacks": True},
        {},
    ]
    for index, kwargs in enumerate(cases):
        instance = module.MockRouterTestingParams.from_kwargs(kwargs)
        result["case_" + str(index)] = {
            "fallbacks": plain(instance.mock_testing_fallbacks),
            "context": plain(instance.mock_testing_context_fallbacks),
            "content": plain(instance.mock_testing_content_policy_fallbacks),
            "remaining": plain(kwargs),
        }
    return result


def constants_results():
    module = load()
    return {
        "special": plain(module.SPECIAL_MODEL_INFO_PARAMS),
        "environments": plain(module.VALID_LITELLM_ENVIRONMENTS),
        "auth_valid": outcome(
            lambda: TypeAdapter(module.CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS).validate_python(
                ["os.environ/X", {"api_base": "b"}]
            )
        ),
        "auth_invalid": outcome(
            lambda: TypeAdapter(module.CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS).validate_python([1])
        ),
        "checks_valid": outcome(
            lambda: TypeAdapter(module.OptionalPreCallChecks).validate_python(
                ["prompt_caching", "session_affinity"]
            )
        ),
        "checks_invalid": outcome(
            lambda: TypeAdapter(module.OptionalPreCallChecks).validate_python(["bogus"])
        ),
    }


def accessor_results():
    module = load()
    return {
        "params": accessors(module.GenericLiteLLMParams(api_key="k"), "api_key"),
        "litellm_params": accessors(module.LiteLLM_Params(model="m"), "model"),
        "model_info": accessors(module.ModelInfo(id="x", base_model="b"), "base_model"),
    }


if mode == "describe":
    module = load()
    result = {
        item: describe(getattr(module, item)) if hasattr(module, item) else {"kind": "missing"}
        for item in spec
    }
elif mode == "public":
    result = {"names": public_names(load())}
elif mode == "all":
    module = load()
    names = list(module.__all__)
    result = {
        "names": names,
        "missing": [item for item in names if not hasattr(module, item)],
        "duplicates": sorted({item for item in names if names.count(item) > 1}),
    }
elif mode == "signatures":
    result = signatures(spec)
elif mode == "model_info":
    result = model_info_results()
elif mode == "params":
    result = params_results()
elif mode == "deployment":
    result = deployment_results()
elif mode == "extra":
    result = extra_results()
elif mode == "groups":
    result = groups_results()
elif mode == "errors":
    result = errors_results()
elif mode == "routing_group":
    result = routing_group_results()
elif mode == "access_window":
    result = access_window_results()
elif mode == "adaptive":
    result = adaptive_results()
elif mode == "mock":
    result = mock_results()
elif mode == "constants":
    result = constants_results()
elif mode == "accessors":
    result = accessor_results()
elif mode == "mock_passthrough":
    module = load()
    kwargs = {"mock_testing_fallbacks": 1, "mock_testing_context_fallbacks": ["x"]}
    instance = module.MockRouterTestingParams.from_kwargs(kwargs)
    result = {
        "fallbacks": plain(instance.mock_testing_fallbacks),
        "context": plain(instance.mock_testing_context_fallbacks),
        "remaining": plain(kwargs),
    }
elif mode == "enum_strings":
    module = load()
    result = {
        "error_type": str(module.RouterErrorTypes.rate_limit_error),
        "error_value": module.RouterErrorTypes.rate_limit_error.value,
        "request_type": str(module.RequestType.CODE_GENERATION),
        "request_value": module.RequestType.CODE_GENERATION.value,
    }
elif mode == "class_names":
    module = load()
    result = {
        "params": module.LiteLLM_Params.__name__,
        "update_params": module.updateLiteLLMParams.__name__,
        "update_deployment": module.updateDeployment.__name__,
        "basic_limit": module.RouterRateLimitErrorBasic.__name__,
    }
elif mode == "type_deviations":
    module = load()
    result = {
        "timeout_object": outcome(
            lambda: type(
                module.GenericLiteLLMParams(timeout=httpx.Timeout(10.0)).timeout
            ).__name__
        ),
        "mock_lambda": outcome(
            lambda: type(
                module.GenericLiteLLMParams(mock_response=lambda: 1).mock_response
            ).__name__
        ),
    }
print(json.dumps(result))
"""

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
SIGNATURE: Final = TypeAdapter(list[list[JsonValue]])
NAMES: Final = TypeAdapter(list[str])
CLASS_NAME_DEVIATIONS: Final = {"RouterRateLimitErrorBasic"}


def run_probe(
    prefix: str, module_name: str, mode: str, spec: Sequence[str] | None = None
) -> dict[str, JsonValue]:
    done = subprocess.run(
        [sys.executable, "-c", PROBE, prefix, module_name, mode, json.dumps(spec)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return OBJECT.validate_json(done.stdout)


def assert_same(mode: str, spec: Sequence[str] | None = None) -> None:
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, mode, spec)
    ours = run_probe(OURS_PREFIX, OURS_MODULE, mode, spec)
    assert ours == theirs


def litellm_public_names() -> list[str]:
    names = NAMES.validate_python(
        run_probe(LITELLM_PREFIX, LITELLM_MODULE, "public")["names"]
    )
    return [item for item in names if item not in EXCLUDED]


def test_names_public_names() -> None:
    theirs = litellm_public_names()
    ours = NAMES.validate_python(run_probe(OURS_PREFIX, OURS_MODULE, "all")["names"])
    assert len(ours) == len(set(ours))
    assert sorted(ours) == sorted(theirs)


def test_names_all_names_resolve() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "all")
    assert result["missing"] == []
    assert result["duplicates"] == []


def test_names_descriptions() -> None:
    spec = [name for name in litellm_public_names() if name not in CLASS_NAME_DEVIATIONS]
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "describe", spec)
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "describe", spec)
    assert all(item != {"kind": "missing"} for item in theirs.values())
    assert ours == theirs


def test_names_protocol_signatures() -> None:
    protocols = [name for name in PROTOCOLS if name != "PreRoutingStrategy"]
    assert len(protocols) == len(PROTOCOLS) - 1
    assert_same("signatures", protocols)


def test_names_strategy_signature() -> None:
    spec = ["PreRoutingStrategy"]
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "signatures", spec)
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "signatures", spec)
    assert ours["PreRoutingStrategy.async_pre_routing_hook"] == [
        ["self", "POSITIONAL_OR_KEYWORD", "_empty"],
        ["model", "POSITIONAL_OR_KEYWORD", "_empty"],
        ["request_kwargs", "POSITIONAL_OR_KEYWORD", "_empty"],
        ["messages", "POSITIONAL_OR_KEYWORD", None],
        ["input_value", "POSITIONAL_OR_KEYWORD", None],
        ["specific_deployment", "KEYWORD_ONLY", False],
    ]
    assert theirs["PreRoutingStrategy.async_pre_routing_hook"] == [
        ["self", "POSITIONAL_OR_KEYWORD", "_empty"],
        ["model", "POSITIONAL_OR_KEYWORD", "_empty"],
        ["request_kwargs", "POSITIONAL_OR_KEYWORD", "_empty"],
        ["messages", "POSITIONAL_OR_KEYWORD", None],
        ["input", "POSITIONAL_OR_KEYWORD", None],
        ["specific_deployment", "POSITIONAL_OR_KEYWORD", False],
    ]


def test_names_error_signatures() -> None:
    spec = ["RouterRateLimitError", "RouterRateLimitErrorBasic"]
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "signatures", spec)
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "signatures", spec)
    basic = "RouterRateLimitErrorBasic.__init__"
    assert ours[basic] == theirs[basic]
    limit = "RouterRateLimitError.__init__"
    ours_limit = SIGNATURE.validate_python(ours[limit])
    theirs_limit = SIGNATURE.validate_python(theirs[limit])
    assert [item[0] for item in ours_limit] == [item[0] for item in theirs_limit]
    assert [item[2] for item in ours_limit] == [item[2] for item in theirs_limit]
    ours_kinds = [item[1] for item in ours_limit[2:]]
    theirs_kinds = [item[1] for item in theirs_limit[2:]]
    assert set(NAMES.validate_python(ours_kinds)) == {"KEYWORD_ONLY"}
    assert set(NAMES.validate_python(theirs_kinds)) == {"POSITIONAL_OR_KEYWORD"}


def test_names_enum_strings() -> None:
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "enum_strings")
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "enum_strings")
    assert ours["error_type"] == "RouterErrorTypes.rate_limit_error"
    assert ours["error_value"] == "rate_limit_error"
    assert ours["request_type"] == "RequestType.CODE_GENERATION"
    assert ours["request_value"] == "code_generation"
    assert ours == theirs


def test_names_class_names() -> None:
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "class_names")
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "class_names")
    assert ours == {
        "params": "LiteLLMParams",
        "update_params": "UpdateLiteLLMParams",
        "update_deployment": "UpdateDeployment",
        "basic_limit": "RouterBasicRateLimitError",
    }
    assert theirs == {
        "params": "LiteLLM_Params",
        "update_params": "updateLiteLLMParams",
        "update_deployment": "updateDeployment",
        "basic_limit": "RouterRateLimitErrorBasic",
    }


def test_names_sibling_types() -> None:
    spec = list(SIBLING_TYPES)
    theirs = run_probe(LITELLM_PREFIX, "utils", "describe", spec)
    ours = run_probe(OURS_PREFIX + ".types", "utils", "describe", spec)
    assert all(item != {"kind": "missing"} for item in ours.values())
    assert ours == theirs


def test_behavior_model_info() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "model_info")
    assert result["generated_id"] == {"str": True, "length": GENERATED_ID_LENGTH}
    assert result["int_id"] == {"ok": "5"}
    assert OBJECT.validate_python(result["float_id"])["validation"] != []
    assert_same("model_info")


def test_behavior_params() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "params")
    assert OBJECT.validate_python(result["reserved"])["extra"] == {}
    assert result["drop_3"] == {"value": True, "type": "bool"}
    assert result["drop_9"] == {"value": "bogus", "type": "str"}
    assert result["drop_10"] == {"value": None, "type": "NoneType"}
    headers = OBJECT.validate_python(result["headers"])
    for header in FORBIDDEN_HEADERS:
        assert headers[header] == {
            "validation": [["value_error", ["provider_affinity_header"], FORBIDDEN_HEADER_MESSAGE]]
        }, header
    assert headers["X-Api-Key"] == {
        "validation": [["value_error", ["provider_affinity_header"], FORBIDDEN_HEADER_MESSAGE]]
    }
    assert headers["X-Session"] == {"ok": "X-Session"}
    for header in ("bad header", ""):
        assert headers[header] == {
            "validation": [["value_error", ["provider_affinity_header"], HEADER_NAME_MESSAGE]]
        }, header
    assert result["header_none"] == {"ok": None}
    assert_same("params")


def test_behavior_deployment() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "deployment")
    assert result["mirror"] == {"input": 0.5, "output": 0.25}
    assert result["dict_params_mirror"] is None
    assert result["dict_info_id"] == {"str": True, "length": GENERATED_ID_LENGTH}
    assert result["bare_id"] == {"str": True, "length": GENERATED_ID_LENGTH}
    assert_same("deployment")


def test_behavior_extra_keys() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "extra")
    assert len(result) == EXTRA_MODELS
    for name, item in result.items():
        entry = OBJECT.validate_python(item)
        assert entry["has_unknown"] is False, name
        assert entry["contains"] is False, name
    assert_same("extra")


def test_behavior_model_group_info() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "groups")
    assert result["vision_0"] == {"ok": False}
    assert result["shared_default"] == {"first": ["temperature"], "second": []}
    for bool_name in (
        "supports_parallel_function_calling",
        "supports_vision",
        "supports_web_search",
        "supports_url_context",
        "supports_reasoning",
        "supports_function_calling",
        "supports_fast_mode",
    ):
        assert result["null_" + bool_name] == {"ok": False}, bool_name
    assert OBJECT.validate_python(result["mode_invalid"])["validation"] != []
    assert_same("groups")


def test_behavior_errors() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "errors")
    assert result["basic"] == {
        "message": "No deployments available for selected model.",
        "model": "m",
        "value_error": True,
    }
    assert_same("errors")


def test_behavior_routing_group() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "routing_group")
    valid = OBJECT.validate_python(OBJECT.validate_python(result["valid"])["ok"])
    assert valid["model_priorities"] == {"a": 1, "b": 2}
    assert_same("routing_group")


def test_behavior_access_window() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "access_window")
    valid = OBJECT.validate_python(OBJECT.validate_python(result["valid"])["ok"])
    assert valid["timezone"] == "Europe/Berlin"
    assert_same("access_window")


def test_behavior_adaptive_weights() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "adaptive")
    assert result["default"] == {"ok": {"quality": 0.7, "cost": 0.3}}
    assert OBJECT.validate_python(result["cost_only"])["validation"] != []
    assert_same("adaptive")


def test_behavior_mock() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "mock")
    first = OBJECT.validate_python(result["case_0"])
    assert first["fallbacks"] is True
    assert first["context"] is False
    assert first["content"] is None
    assert first["remaining"] == {"other": 1}
    assert_same("mock")


def test_behavior_mock_passthrough() -> None:
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "mock_passthrough")
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "mock_passthrough")
    assert ours == {"fallbacks": None, "context": None, "remaining": {}}
    assert theirs == {"fallbacks": MOCK_INT_MARKER, "context": ["x"], "remaining": {}}


def test_behavior_type_deviations() -> None:
    ours = run_probe(OURS_PREFIX, OURS_MODULE, "type_deviations")
    theirs = run_probe(LITELLM_PREFIX, LITELLM_MODULE, "type_deviations")
    assert OBJECT.validate_python(ours["timeout_object"])["validation"] != []
    assert OBJECT.validate_python(ours["mock_lambda"])["validation"] != []
    assert theirs == {"timeout_object": {"ok": "Timeout"}, "mock_lambda": {"ok": "function"}}


def test_behavior_constants() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "constants")
    assert result["special"] == [
        "input_cost_per_token",
        "output_cost_per_token",
        "input_cost_per_character",
        "output_cost_per_character",
        "cache_read_input_token_cost",
        "cache_creation_input_token_cost",
        "tiered_pricing",
    ]
    assert OBJECT.validate_python(result["checks_invalid"])["validation"] != []
    assert_same("constants")


def test_behavior_dict_accessors() -> None:
    result = run_probe(OURS_PREFIX, OURS_MODULE, "accessors")
    assert OBJECT.validate_python(result["params"])["get_default"] == {"ok": "d"}
    assert OBJECT.validate_python(result["model_info"])["custom"] == {"ok": 7}
    assert_same("accessors")
