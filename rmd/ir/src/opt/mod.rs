mod canonicalize_terminators;
mod eliminate_unreachable_blocks;
mod simplify_phis;
mod sparse_constant_propagation;

pub use canonicalize_terminators::canonicalize_terminators;
pub use eliminate_unreachable_blocks::eliminate_unreachable_blocks;
pub use simplify_phis::simplify_phis;
pub use sparse_constant_propagation::sparse_constant_propagation;
