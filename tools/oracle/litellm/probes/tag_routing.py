from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class TagRouting(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "tag_routing is not built")
