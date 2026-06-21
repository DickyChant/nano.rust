//! Demo crate that compiles a producer generated from `nano-spec`.

#![allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]

include!(concat!(env!("OUT_DIR"), "/generated_muon.rs"));

#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]
pub mod selection_all {
    include!(concat!(env!("OUT_DIR"), "/generated_selection_all.rs"));
}

#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]
pub mod selection_charge_balance {
    include!(concat!(
        env!("OUT_DIR"),
        "/generated_selection_charge_balance.rs"
    ));
}

#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]
pub mod selection_sip3d {
    include!(concat!(env!("OUT_DIR"), "/generated_selection_sip3d.rs"));
}

#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]
pub mod selection_pair_dr {
    include!(concat!(env!("OUT_DIR"), "/generated_selection_pair_dr.rs"));
}
