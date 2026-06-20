//! Write a small NanoAOD-like file (scalars + jagged Muon_pt) for interop checks.
use nano_rootio::write::{write_tree, Branch};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/nr_demo.root".into());
    let n_muon = vec![0u32, 1, 2, 2, 1, 3];
    let muon_pt = vec![
        vec![],
        vec![25.0f32],
        vec![40.0, 30.0],
        vec![55.0, 12.0],
        vec![60.0],
        vec![70.0, 20.0, 11.0],
    ];
    let met_pt = vec![10.0f32, 20.0, 30.0, 40.0, 50.0, 60.0];
    write_tree(
        &path,
        "Events",
        &[
            Branch::u32("nMuon", n_muon),
            Branch::vec_f32("Muon_pt", muon_pt),
            Branch::f32("MET_pt", met_pt),
        ],
    )
    .expect("write_tree");
    eprintln!("wrote {path}");
}
