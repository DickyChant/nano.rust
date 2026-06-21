//! Demo crate that compiles minimal Higgs four-lepton producers generated from `nano-spec`.

#![allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]

#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]
pub mod higgs4mu {
    include!(concat!(env!("OUT_DIR"), "/generated_higgs4mu.rs"));
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
pub mod higgs2e2mu {
    include!(concat!(env!("OUT_DIR"), "/generated_higgs2e2mu.rs"));
}
