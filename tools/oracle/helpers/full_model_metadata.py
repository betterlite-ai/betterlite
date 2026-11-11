import copy
import io
from contextlib import redirect_stdout
from functools import lru_cache, partial
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import httpx
import litellm
import litellm.utils as litellm_utils
from litellm.constants import DEFAULT_MAX_LRU_CACHE_SIZE
from litellm.exceptions import ModelNotMappedError
from litellm.litellm_core_utils import get_model_cost_map
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.lemonade.chat.transformation import LemonadeChatConfig
from litellm.llms.ollama.common_utils import OllamaModelInfo
from litellm.types.utils import ModelInfoBase
from litellm.utils import get_model_cost_mutation_generation
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Outcome: TypeAlias = tuple[Object, bool]


class ParameterScript(TypedDict):
    kind: Literal["value", "absent", "error"]
    body: NotRequired[list[str] | None]


class CallStep(TypedDict):
    op: Literal["call"]
    model: str
    custom_llm_provider: NotRequired[str | None]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    parameters: NotRequired[ParameterScript]


class HelperCallStep(TypedDict):
    op: Literal["helper_call"]
    model: str
    custom_llm_provider: NotRequired[str | None]
    api_base: NotRequired[str | None]


class MutateStep(TypedDict):
    op: Literal["mutate"]
    record: int
    field: str
    value: Json


class InvalidateStep(TypedDict):
    op: Literal["invalidate"]


class ReplaceStep(TypedDict):
    op: Literal["replace"]
    key: str
    entry: Object


Step: TypeAlias = CallStep | HelperCallStep | MutateStep | InvalidateStep | ReplaceStep
LookupStep: TypeAlias = CallStep | HelperCallStep


class Fixture(TypedDict):
    model_cost: NotRequired[dict[str, Object]]
    cache_maxsize: NotRequired[int]
    dynamic: NotRequired[Object | None]


class Input(TypedDict):
    fixture: Fixture
    steps: list[Step]


class ParameterQueryError(Exception):
    pass


class OfflineTransport(httpx.BaseTransport):
    @override
    def handle_request(self, request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("no fixture transport", request=request)


class Driver:
    def __init__(self, fixture: Fixture) -> None:
        costs = fixture.get("model_cost")
        self.costs: dict[str, Object] = {} if costs is None else costs
        self.dynamic = fixture.get("dynamic")
        self.calls: list[Json] = []
        self.records: list[Object] = []
        self.script: ParameterScript | None = None

    def dynamic_info(
        self, adapter: str, *, model: str, api_base: str | None, api_key: str | None
    ) -> Object | None:
        self.calls.append(
            {
                "kind": "dynamic",
                "adapter": adapter,
                "model": model,
                "api_base": api_base,
                "authenticated": api_key is not None,
            }
        )
        return copy.deepcopy(self.dynamic)

    def probe(self, model: str, custom_llm_provider: str | None = None) -> list[str] | None:
        script = self.script
        if script is not None and script["kind"] == "error":
            self.calls.append(
                {
                    "kind": "parameters",
                    "model": model,
                    "provider": custom_llm_provider,
                    "raised": True,
                }
            )
            raise ParameterQueryError
        body = None if script is None else script.get("body")
        shown: Json = None if body is None else [*body]
        self.calls.append(
            {
                "kind": "parameters",
                "model": model,
                "provider": custom_llm_provider,
                "answer": shown,
            }
        )
        return body


def install(driver: Driver, maxsize: int) -> None:
    litellm.model_cost = driver.costs
    litellm.module_level_client = HTTPHandler(client=httpx.Client(transport=OfflineTransport()))
    OllamaModelInfo.get_model_info = partial(driver.dynamic_info, "ollama")  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L9321
    LemonadeChatConfig.get_model_info = partial(driver.dynamic_info, "lemonade")  # type: ignore[method-assign, assignment, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L9329
    litellm.get_supported_openai_params = driver.probe
    litellm.vertex_llama3_models = set()
    litellm.vertex_mistral_models = set()
    litellm.vertex_ai_ai21_models = set()
    add_known = litellm.add_known_models  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1009
    add_known(driver.costs)
    cache = lru_cache(maxsize=maxsize)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5870
    litellm_utils._cached_get_model_info_helper = cache(litellm_utils._get_model_info_helper)  # type: ignore[misc]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5870
    litellm_utils._cached_get_model_info = cache(litellm_utils._build_model_info)  # type: ignore[misc]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L6407


def lookup(step: LookupStep) -> ModelInfoBase:
    model = step["model"]
    provider = step.get("custom_llm_provider")
    api_base = step.get("api_base")
    if step["op"] == "helper_call":
        return litellm_utils._cached_get_model_info_helper(model, provider, api_base)  # type: ignore[misc]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5871
    return litellm_utils.get_model_info(model, provider, api_base, step.get("api_key"))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L6416


def outcome(step: LookupStep) -> Outcome:
    try:
        found = lookup(step)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L271
    except ParameterQueryError:
        return {"raised": "parameters"}, True
    except ModelNotMappedError:
        return {"raised": "not_mapped"}, True
    except TypeError as error:
        duplicate = "multiple values" in str(error)
        return {"raised": "duplicate_supported_openai_params" if duplicate else "record"}, True
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L6377
        return {"raised": "failed"}, True
    live = cast("Object", found)  # fmt: skip  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
    return live, False


def cache_json(info: tuple[int, int, int | None, int]) -> Json:
    hits, misses, maxsize, currsize = info
    return {"hits": hits, "misses": misses, "maxsize": maxsize, "currsize": currsize}


def caches_json() -> Json:
    full = litellm_utils._cached_get_model_info.cache_info()  # type: ignore[misc]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L6407
    helper = litellm_utils._cached_get_model_info_helper.cache_info()  # type: ignore[misc]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5870
    return {"full": cache_json(full), "helper": cache_json(helper)}


def lookup_step(driver: Driver, step: LookupStep) -> Json:
    if step["op"] == "call":
        driver.script = step.get("parameters")
    start = len(driver.calls)
    result, failed = outcome(step)
    calls = driver.calls[start:]
    if failed:
        return {"op": step["op"], "result": result, "calls": calls, "caches": caches_json()}
    # records keeps the live dict, the observation keeps a snapshot of it
    record = len(driver.records)
    driver.records.append(result)
    return {
        "op": step["op"],
        "result": copy.deepcopy(result),
        "record": record,
        "calls": calls,
        "caches": caches_json(),
    }


def apply_step(driver: Driver, step: Step) -> Json:
    if step["op"] == "call" or step["op"] == "helper_call":
        return lookup_step(driver, step)
    if step["op"] == "mutate":
        driver.records[step["record"]][step["field"]] = step["value"]
        return {"op": "mutate", "record": step["record"], "field": step["field"]}
    if step["op"] == "invalidate":
        adopt = get_model_cost_map.adopt_model_cost_map  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/get_model_cost_map.py#L556
        adopt(driver.costs)
        return {
            "op": "invalidate",
            "generation": get_model_cost_mutation_generation(),
            "caches": caches_json(),
        }
    driver.costs[step["key"]] = step["entry"]
    return {"op": "replace", "key": step["key"]}


def run(input_value: Input) -> Json:
    fixture = input_value["fixture"]
    driver = Driver(fixture)
    maxsize = fixture.get("cache_maxsize")
    with redirect_stdout(io.StringIO()):
        install(driver, DEFAULT_MAX_LRU_CACHE_SIZE if maxsize is None else maxsize)
        return [apply_step(driver, step) for step in input_value["steps"]]
