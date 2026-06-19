//! nano-core — framework-level abstractions for the nano.rust event loop.
//!
//! Ports the C++ `include/nano/core` model (see `docs/framework-plan.md`):
//! `Event`, `Collection`/`ObjectView`, the branch schema, and the
//! object/attribute grouping rule (`Prefix_attr` -> object `Prefix`, attr `attr`).
//! Implementations land here as the port proceeds; this is the green skeleton.

/// Split a NanoAOD branch name into `(object, attribute)` per the grouping rule.
///
/// Vector branches named `Prefix_attr` map to object `Prefix` with attribute
/// `attr`. Names without an underscore (or non-collection branches such as
/// `Flag_goodVertices`, handled by the caller via the schema) are event-level
/// and return `None` here.
///
/// ```
/// use nano_core::split_branch_name;
/// assert_eq!(split_branch_name("FatJet_pt"), Some(("FatJet", "pt")));
/// assert_eq!(split_branch_name("Muon_miniPFRelIso_all"), Some(("Muon", "miniPFRelIso_all")));
/// assert_eq!(split_branch_name("MET"), None);
/// ```
pub fn split_branch_name(branch: &str) -> Option<(&str, &str)> {
    branch.split_once('_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_object_branches() {
        assert_eq!(split_branch_name("FatJet_pt"), Some(("FatJet", "pt")));
        assert_eq!(
            split_branch_name("Muon_miniPFRelIso_all"),
            Some(("Muon", "miniPFRelIso_all"))
        );
        assert_eq!(split_branch_name("MET"), None);
    }
}
