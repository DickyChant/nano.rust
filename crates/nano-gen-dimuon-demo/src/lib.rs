//! Demo crate that compiles a dimuon producer generated from `nano-spec`.

#![allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]

include!(concat!(env!("OUT_DIR"), "/generated_dimuon.rs"));
