use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_rootio::write::{write_tree, Branch};
use nano_validate::{
    compare_root_files, BranchPresence, CompareOptions, ComparisonStatus, ValueKind,
};

#[test]
fn identical_files_pass_with_zero_mismatches() {
    let fixture = Fixture::new("identical");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_validation_file(&reference, &[Branch::f32("pt", vec![1.0, 2.0, 3.0])]);
    write_validation_file(&candidate, &[Branch::f32("pt", vec![1.0, 2.0, 3.0])]);

    let report = compare_root_files(&reference, &candidate, &CompareOptions::default()).unwrap();

    assert_eq!(report.status, ComparisonStatus::Pass);
    assert!(report.entry_count_match);
    assert!(report
        .branches
        .iter()
        .all(|branch| branch.n_mismatched == 0));
}

#[test]
fn perturbed_branch_beyond_tolerance_is_detected() {
    let fixture = Fixture::new("perturbed");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_validation_file(&reference, &[Branch::f32("pt", vec![1.0, 2.0, 3.0])]);
    write_validation_file(&candidate, &[Branch::f32("pt", vec![1.0, 2.2, 3.0])]);

    let report = compare_root_files(
        &reference,
        &candidate,
        &CompareOptions {
            rtol: 1e-6,
            atol: 1e-6,
            ..CompareOptions::default()
        },
    )
    .unwrap();

    let pt = branch(&report.branches, "pt");
    assert_eq!(report.status, ComparisonStatus::Fail);
    assert_eq!(pt.value_kind, Some(ValueKind::ScalarF32));
    assert_eq!(pt.n_compared, 3);
    assert_eq!(pt.n_mismatched, 1);
    assert_eq!(pt.first_mismatches[0].entry, 1);
    assert!(pt.max_abs_diff.unwrap() > 0.19);
}

#[test]
fn branch_present_in_only_one_file_is_reported() {
    let fixture = Fixture::new("missing-branch");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_validation_file(
        &reference,
        &[
            Branch::i32("run", vec![1, 1, 1]),
            Branch::f32("ref_only", vec![1.0, 2.0, 3.0]),
        ],
    );
    write_validation_file(
        &candidate,
        &[
            Branch::i32("run", vec![1, 1, 1]),
            Branch::f32("candidate_only", vec![1.0, 2.0, 3.0]),
        ],
    );

    let report = compare_root_files(&reference, &candidate, &CompareOptions::default()).unwrap();

    assert_eq!(report.status, ComparisonStatus::Fail);
    assert_eq!(
        branch(&report.branches, "ref_only").presence,
        BranchPresence::OnlyInReference
    );
    assert_eq!(
        branch(&report.branches, "candidate_only").presence,
        BranchPresence::OnlyInCandidate
    );
}

#[test]
fn within_tolerance_float_jitter_passes() {
    let fixture = Fixture::new("within-tolerance");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_validation_file(
        &reference,
        &[
            Branch::u32("nMuon", vec![2, 1]),
            Branch::vec_f32("Muon_pt", vec![vec![10.0, 20.0], vec![30.0]]),
        ],
    );
    write_validation_file(
        &candidate,
        &[
            Branch::u32("nMuon", vec![2, 1]),
            Branch::vec_f32("Muon_pt", vec![vec![10.00001, 20.00001], vec![30.00001]]),
        ],
    );

    let report = compare_root_files(
        &reference,
        &candidate,
        &CompareOptions {
            rtol: 1e-3,
            atol: 1e-4,
            ..CompareOptions::default()
        },
    )
    .unwrap();

    let muon_pt = branch(&report.branches, "Muon_pt");
    assert_eq!(report.status, ComparisonStatus::Pass);
    assert_eq!(muon_pt.value_kind, Some(ValueKind::JaggedF32));
    assert_eq!(muon_pt.n_compared, 3);
    assert_eq!(muon_pt.n_mismatched, 0);
    assert!(muon_pt.max_abs_diff.unwrap() > 0.0);
}

fn write_validation_file(path: &Path, extra_branches: &[Branch]) {
    write_tree(path, "Events", extra_branches).unwrap();
}

fn branch<'a>(
    branches: &'a [nano_validate::BranchComparison],
    name: &str,
) -> &'a nano_validate::BranchComparison {
    branches
        .iter()
        .find(|branch| branch.name == name)
        .unwrap_or_else(|| panic!("missing branch {name} in {branches:#?}"))
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nano-validate-{}-{timestamp}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}
