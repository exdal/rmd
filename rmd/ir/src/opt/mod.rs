mod canonicalize_terminators;
mod eliminate_dead_code;
mod eliminate_unreachable_blocks;
mod merge_linear_blocks;
mod simplify_phis;
mod sparse_constant_propagation;

pub use canonicalize_terminators::canonicalize_terminators;
pub use eliminate_dead_code::eliminate_dead_code;
pub use eliminate_unreachable_blocks::eliminate_unreachable_blocks;
pub use merge_linear_blocks::merge_linear_blocks;
pub use simplify_phis::simplify_phis;
pub use sparse_constant_propagation::sparse_constant_propagation;
