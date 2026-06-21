//! Demo crate that compiles minimal Higgs four-lepton producers generated from `nano-spec`.

pub mod higgs4mu {
    include!(concat!(env!("OUT_DIR"), "/generated_higgs4mu.rs"));
}

pub mod higgs2e2mu {
    include!(concat!(env!("OUT_DIR"), "/generated_higgs2e2mu.rs"));
}
