//! Demo crate that compiles minimal Higgs four-lepton producers generated from `nano-spec`.
//!
//! The generated channel region markers are ordinary `nano_analysis::Region`
//! types, so the generated histogram structure inherits the same
//! weight-before-fill guarantee as hand-written kernels.
//!
//! ```compile_fail
//! use nano_analysis::{fill, Ev, Hist1D};
//! use nano_core::{BranchColumn, BranchSchema, BranchSpec, Event};
//! use nano_gen_higgs4l_demo::higgs4l_all;
//!
//! let schema = BranchSchema::new(Vec::<BranchSpec>::new()).unwrap();
//! let event = Event::from_columns(schema, Vec::<(String, BranchColumn)>::new(), 0).unwrap();
//! let selected = Ev::new(&event)
//!     .preselect(|_| true)
//!     .unwrap()
//!     .select::<higgs4l_all::four_mu::SignalRegion>(|_| true)
//!     .unwrap();
//! let mut hist = Hist1D::new(1, 0.0, 1.0);
//! fill::<higgs4l_all::four_mu::SignalRegion, nano_analysis::Nominal>(
//!     &mut hist, &selected, 0.5,
//! );
//! ```
//!
//! ```compile_fail
//! use nano_analysis::{fill, Ev, EventWeight, Hist1D};
//! use nano_core::{BranchColumn, BranchSchema, BranchSpec, Event};
//! use nano_gen_higgs4l_demo::higgs4l_all;
//!
//! let schema = BranchSchema::new(Vec::<BranchSpec>::new()).unwrap();
//! let event = Event::from_columns(schema, Vec::<(String, BranchColumn)>::new(), 0).unwrap();
//! let weighted_four_e = Ev::new(&event)
//!     .preselect(|_| true)
//!     .unwrap()
//!     .select::<higgs4l_all::four_e::SignalRegion>(|_| true)
//!     .unwrap()
//!     .weight(EventWeight::nominal());
//! let mut hist = Hist1D::new(1, 0.0, 1.0);
//! fill::<higgs4l_all::four_mu::SignalRegion, nano_analysis::Nominal>(
//!     &mut hist, &weighted_four_e, 0.5,
//! );
//! ```

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
pub mod higgs4e {
    include!(concat!(env!("OUT_DIR"), "/generated_higgs4e.rs"));
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

#[allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]
pub mod higgs4l_all {
    include!(concat!(env!("OUT_DIR"), "/generated_higgs4l_all.rs"));
}
