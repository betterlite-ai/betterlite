from litellm.proxy.lens.repository import LensRepository

from .calls import output, read_call, update_call, worker_call
from .model import Input, Output
from .store import Store


def run(input_value: Input) -> Output:
    call = input_value["call"]
    conflicts = call.get("conflicts")
    seed = input_value.get("seed")
    store = Store({} if seed is None else seed, 0 if conflicts is None else conflicts)
    repository = LensRepository(store)
    read = read_call(repository, call)
    if read is not None:
        result, transform_calls = read
        return output(store, result, transform_calls)
    if call["operation"] == "update":
        result, transform_calls = update_call(repository, call)
        return output(store, result, transform_calls)
    return output(store, worker_call(repository, call), 0)
