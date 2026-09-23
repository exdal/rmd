mod canonicalize_terminators;
mod eliminate_dead_code;
mod eliminate_dead_stores;
mod eliminate_forwarding_blocks;
mod eliminate_unreachable_blocks;
mod merge_linear_blocks;
mod peephole;
mod simplify_phis;
mod sparse_constant_propagation;
mod value_facts;

use std::time::{Duration, Instant};

pub use canonicalize_terminators::canonicalize_terminators;
pub use eliminate_dead_code::eliminate_dead_code;
pub use eliminate_dead_stores::eliminate_dead_stores;
pub use eliminate_forwarding_blocks::eliminate_forwarding_blocks;
pub use eliminate_unreachable_blocks::eliminate_unreachable_blocks;
pub use merge_linear_blocks::merge_linear_blocks;
pub use peephole::peephole;
pub use simplify_phis::simplify_phis;
pub use sparse_constant_propagation::sparse_constant_propagation;

use crate::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum OptimizationPass {
    SimplifyPhis,
    SparseConstantPropagation,
    EliminateUnreachableBlocks,
    SimplifyPhisAfterCfg,
    Peephole,
    EliminateDeadStores,
    EliminateDeadCode,
    MergeLinearBlocks,
    EliminateForwardingBlocks,
}

impl OptimizationPass {
    pub const ALL: [Self; Self::COUNT] = [
        Self::SimplifyPhis,
        Self::SparseConstantPropagation,
        Self::EliminateUnreachableBlocks,
        Self::SimplifyPhisAfterCfg,
        Self::Peephole,
        Self::EliminateDeadStores,
        Self::EliminateDeadCode,
        Self::MergeLinearBlocks,
        Self::EliminateForwardingBlocks,
    ];
    const COUNT: usize = 9;

    pub const fn label(self) -> &'static str {
        match self {
            Self::SimplifyPhis => "Simplify phis",
            Self::SparseConstantPropagation => "Sparse constant propagation",
            Self::EliminateUnreachableBlocks => "Eliminate unreachable blocks",
            Self::SimplifyPhisAfterCfg => "Simplify phis after CFG cleanup",
            Self::Peephole => "Peephole",
            Self::EliminateDeadStores => "Eliminate dead stores",
            Self::EliminateDeadCode => "Eliminate dead code",
            Self::MergeLinearBlocks => "Merge linear blocks",
            Self::EliminateForwardingBlocks => "Eliminate forwarding blocks",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationTimings {
    elapsed: [Duration; OptimizationPass::COUNT],
    samples: u32,
}

impl Default for OptimizationTimings {
    fn default() -> Self {
        Self {
            elapsed: [Duration::ZERO; OptimizationPass::COUNT],
            samples: 0,
        }
    }
}

impl OptimizationTimings {
    fn single_run() -> Self {
        Self {
            samples: 1,
            ..Self::default()
        }
    }

    fn measure(&mut self, pass: OptimizationPass, operation: impl FnOnce()) {
        let started = Instant::now();
        operation();
        self.elapsed[pass as usize] += started.elapsed();
    }

    pub fn merge(&mut self, other: Self) {
        self.samples = self.samples.saturating_add(other.samples);
        for (elapsed, additional) in self.elapsed.iter_mut().zip(other.elapsed) {
            *elapsed += additional;
        }
    }

    pub const fn samples(self) -> u32 { self.samples }

    pub fn average(self, pass: OptimizationPass) -> Duration {
        self.elapsed[pass as usize]
            .checked_div(self.samples)
            .unwrap_or_default()
    }

    pub fn average_total(self) -> Duration {
        self.elapsed
            .into_iter()
            .sum::<Duration>()
            .checked_div(self.samples)
            .unwrap_or_default()
    }
}

pub fn run_optimizations(module: &mut Module, enabled: bool) -> OptimizationTimings {
    let mut timings = OptimizationTimings::single_run();

    if !enabled {
        timings.measure(OptimizationPass::SimplifyPhis, || simplify_phis(module));

        return timings;
    }

    timings.measure(OptimizationPass::SimplifyPhis, || simplify_phis(module));
    timings.measure(OptimizationPass::SparseConstantPropagation, || {
        sparse_constant_propagation(module)
    });
    timings.measure(OptimizationPass::EliminateUnreachableBlocks, || {
        eliminate_unreachable_blocks(module)
    });
    timings.measure(OptimizationPass::SimplifyPhisAfterCfg, || simplify_phis(module));
    timings.measure(OptimizationPass::Peephole, || peephole(module));
    timings.measure(OptimizationPass::EliminateDeadStores, || eliminate_dead_stores(module));
    timings.measure(OptimizationPass::EliminateDeadCode, || eliminate_dead_code(module));
    timings.measure(OptimizationPass::MergeLinearBlocks, || merge_linear_blocks(module));
    timings.measure(OptimizationPass::EliminateForwardingBlocks, || {
        eliminate_forwarding_blocks(module)
    });

    timings
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{OptimizationPass, OptimizationTimings};

    #[test]
    fn timing_samples_are_averaged_across_compiler_runs() {
        let mut first = OptimizationTimings::single_run();
        first.elapsed[OptimizationPass::Peephole as usize] = Duration::from_millis(10);
        let mut second = OptimizationTimings::single_run();
        second.elapsed[OptimizationPass::Peephole as usize] = Duration::from_millis(20);
        second.elapsed[OptimizationPass::EliminateDeadCode as usize] = Duration::from_millis(4);

        first.merge(second);

        assert_eq!(first.samples(), 2);
        assert_eq!(first.average(OptimizationPass::Peephole), Duration::from_millis(15));
        assert_eq!(
            first.average(OptimizationPass::EliminateDeadCode),
            Duration::from_millis(2)
        );
        assert_eq!(first.average_total(), Duration::from_millis(17));
    }
}
