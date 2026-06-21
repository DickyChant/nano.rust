//! Demo crate for a generated multijet HT control-region producer.

#![allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]

pub mod reference;

include!(concat!(env!("OUT_DIR"), "/generated_multijet_ht.rs"));
