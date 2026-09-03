// SPDX-License-Identifier: Apache-2.0

//! Per-evaluation resource limits and cancellation. These bound runaway
//! repository code; they are not a proof that a hostile repository is
//! contained.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use starlark::eval::Evaluator;

use crate::bootstrap::Limits;

/// Apply the tick, heap, and call-stack limits and the cancellation flag.
pub fn apply_limits(
    eval: &mut Evaluator<'_, '_, '_>,
    limits: Limits,
    cancel: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    eval.set_max_tick_count(limits.ticks)?;
    eval.set_max_heap_size(limits.heap_bytes)?;
    eval.set_max_callstack_size(limits.call_stack)?;
    let flag = Arc::clone(cancel);
    eval.set_check_cancelled(Box::new(move || flag.load(Ordering::Relaxed)));
    Ok(())
}
