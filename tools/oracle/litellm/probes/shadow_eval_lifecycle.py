from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class ShadowEvalLifecycle(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "shadow_eval_lifecycle is not built")
