//! Demo crate for a generated boosted-muon tagger control-region producer.

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

pub mod mutagger_weight_systematic {
    include!(concat!(
        env!("OUT_DIR"),
        "/generated_mutagger_cr_weight_systematic.rs"
    ));
}

pub mod mutagger_shape_crossing_base {
    include!(concat!(
        env!("OUT_DIR"),
        "/generated_mutagger_shape_crossing_base.rs"
    ));
}

pub mod mutagger_shape_crossing {
    include!(concat!(
        env!("OUT_DIR"),
        "/generated_mutagger_shape_crossing.rs"
    ));
}

include!(concat!(env!("OUT_DIR"), "/generated_mutagger_cr.rs"));
