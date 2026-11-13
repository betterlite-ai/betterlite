from litellm.proxy.lens.repository import LensRepository

from .calls import output, read_lens_call, read_worker_call, write_lens_call, write_worker_call
from .model import Input, Output
from .store import Store


def run(input_value: Input) -> Output:
    call = input_value["call"]
    conflicts = call.get("conflicts")
    seed = input_value.get("seed")
    store = Store({} if seed is None else seed, 0 if conflicts is None else conflicts)
    repository = LensRepository(store)
    operation = call["operation"]
    if operation in ("workers", "worker"):
        return output(store, read_worker_call(repository, call), 0)
    if operation in ("save_worker", "set_worker_billing", "revoke_worker", "heartbeat"):
        return output(store, write_worker_call(repository, call), 0)
    read = read_lens_call(repository, call)
    if read is not None:
        result, transform_calls = read
        return output(store, result, transform_calls)
    result, transform_calls = write_lens_call(repository, call)
    return output(store, result, transform_calls)
