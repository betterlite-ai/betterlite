from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class RoutingReadBatch(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "routing_read_batch is not built")
