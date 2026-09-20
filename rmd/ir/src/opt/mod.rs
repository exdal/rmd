mod canonicalize_terminators;
mod simplify_phis;
mod sparse_constant_propagation;

pub use canonicalize_terminators::canonicalize_terminators;
pub use simplify_phis::simplify_phis;
pub use sparse_constant_propagation::sparse_constant_propagation;
