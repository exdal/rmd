//! Compiler-owned DM definitions and their intrinsic ABI.

pub const CORE_SOURCE: &str = include_str!("../core.dm");
pub const STDDEF_SOURCE: &str = include_str!("../stddef.dm");
pub const DEMIR_SOURCE: &str = include_str!("../demir.dm");

include!(concat!(env!("OUT_DIR"), "/intrinsic.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_intrinsics_preserve_the_declared_abi() {
        assert_eq!(Intrinsic::try_from(100).unwrap(), Intrinsic::WorldNew);
        assert_eq!(Intrinsic::try_from(401).unwrap(), Intrinsic::Call);
        assert_eq!(Intrinsic::try_from(1100).unwrap(), Intrinsic::ListAdd);
        assert!(Intrinsic::try_from(u16::MAX).is_err());
    }

    #[test]
    fn core_owns_the_required_root_types() {
        assert!(CORE_SOURCE.contains("/datum"));
        assert!(CORE_SOURCE.contains("/list"));
        assert!(CORE_SOURCE.contains("/alist"));
        assert!(!DEMIR_SOURCE.contains("\n/datum\n"));
        assert!(!DEMIR_SOURCE.contains("\n/list\n"));
        assert!(!DEMIR_SOURCE.contains("\n/alist\n"));
    }
}
