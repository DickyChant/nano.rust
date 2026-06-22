use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_rootio::write::{write_tree, Branch};
use nano_rootio::{BranchInfo, Error as RootError, RootFile, Scalar, Tree};
use nano_validate::{compare_root_files, CompareOptions, ComparisonStatus};

const SINGLEMU_2018_NANOV9: &str =
    "tests/data/muon_validation/references/singlemu_2018_nanov9_reference.root";
const TTBARFL_2022EE_NANOV12: &str =
    "tests/data/muon_validation/references/ttbarfl_2022EE_nanov12_reference.root";
const TTBARFL_2024_NANOV15: &str =
    "tests/data/muon_validation/references/ttbarfl_2024_nanov15_reference.root";

#[test]
fn singlemu_2018_nanov9_frozen_reference_round_trips_and_compares() {
    let reference = repo_path(SINGLEMU_2018_NANOV9);
    let tree = open_events_tree(&reference);
    assert_eq!(tree.entries(), 954);
    assert_sane_singlemu_values(&tree);

    let branches = read_all_branches(&tree);
    assert_eq!(branches.len(), 122);

    let fixture = Fixture::new("singlemu-2018-nanov9");
    let roundtrip = fixture.path("roundtrip.root");
    write_snapshot(&roundtrip, &branches);

    let roundtrip_tree = open_events_tree(&roundtrip);
    let roundtrip_branches = read_all_branches(&roundtrip_tree);
    assert_same_branch_data(&branches, &roundtrip_branches);

    let options = tight_compare_options();
    let equal = compare_root_files(&reference, &roundtrip, &options).unwrap();
    assert_eq!(equal.status, ComparisonStatus::Pass, "{}", equal.summary());

    let perturbed = fixture.path("perturbed.root");
    let mut perturbed_branches = branches.clone();
    perturb_scalar_f32(&mut perturbed_branches, "met");
    write_snapshot(&perturbed, &perturbed_branches);

    let different = compare_root_files(&reference, &perturbed, &options).unwrap();
    assert_eq!(
        different.status,
        ComparisonStatus::Fail,
        "perturbed branch unexpectedly compared equal:\n{}",
        different.summary()
    );
    let met = different
        .branches
        .iter()
        .find(|branch| branch.name == "met")
        .expect("met comparison");
    assert!(
        met.n_mismatched > 0,
        "met perturbation was not reported:\n{}",
        different.summary()
    );
}

#[test]
#[ignore = "nano-rootio opens this file but RootFile::tree(\"Events\") returns Parse { offset: 0, message: \"unexpected branch object class TLeafF\" }"]
fn ttbarfl_2022ee_nanov12_frozen_reference_documents_reader_gap() {
    assert_events_tree_parse_gap(TTBARFL_2022EE_NANOV12);
}

#[test]
#[ignore = "nano-rootio opens this file but RootFile::tree(\"Events\") returns Parse { offset: 0, message: \"unexpected branch object class TLeafF\" }"]
fn ttbarfl_2024_nanov15_frozen_reference_documents_reader_gap() {
    assert_events_tree_parse_gap(TTBARFL_2024_NANOV15);
}

fn open_events_tree(path: &Path) -> Tree {
    let file = RootFile::open(path).unwrap_or_else(|error| {
        panic!("failed to open {}: {error}", path.display());
    });
    assert!(
        file.objects()
            .iter()
            .any(|object| object.name() == "Events" && object.class() == "TTree"),
        "{} does not list an Events TTree",
        path.display()
    );
    file.tree("Events").unwrap_or_else(|error| {
        panic!("failed to parse Events in {}: {error:?}", path.display());
    })
}

fn assert_sane_singlemu_values(tree: &Tree) {
    let entries = usize::try_from(tree.entries()).expect("non-negative entries");
    let run = tree.read_scalar::<u32>("run").expect("run");
    let event = tree.read_scalar::<u64>("event").expect("event");
    let met = tree.read_scalar::<f32>("met").expect("met");
    let pass_mu_trig = tree.read_scalar::<bool>("passMuTrig").expect("passMuTrig");
    let muon_pt = tree.read_scalar::<f32>("muon_pt").expect("muon_pt");
    let fj_1_pt = tree.read_scalar::<f32>("fj_1_pt").expect("fj_1_pt");

    for (name, len) in [
        ("run", run.len()),
        ("event", event.len()),
        ("met", met.len()),
        ("passMuTrig", pass_mu_trig.len()),
        ("muon_pt", muon_pt.len()),
        ("fj_1_pt", fj_1_pt.len()),
    ] {
        assert_eq!(len, entries, "{name} length differs from Events entries");
    }

    assert!(run.iter().all(|value| *value > 0));
    assert!(event.iter().all(|value| *value > 0));
    assert!(met.iter().any(|value| value.is_finite() && *value > 0.0));
    assert!(muon_pt
        .iter()
        .any(|value| value.is_finite() && *value > 0.0));
    assert!(fj_1_pt
        .iter()
        .any(|value| value.is_finite() && *value > 0.0));
    assert!(pass_mu_trig.iter().any(|value| *value));
}

fn read_all_branches(tree: &Tree) -> Vec<BranchSnapshot> {
    tree.branches()
        .iter()
        .map(|branch| read_branch(tree, branch))
        .collect()
}

fn read_branch(tree: &Tree, branch: &BranchInfo) -> BranchSnapshot {
    let [type_name] = branch.types.as_slice() else {
        panic!(
            "unsupported branch `{}` with type signature {:?}",
            branch.name, branch.types
        );
    };

    macro_rules! read_typed {
        ($ty:ty, $scalar:ident, $jagged:ident) => {{
            match tree.read_scalar::<$ty>(&branch.name) {
                Ok(values) => BranchSnapshot::$scalar(branch.name.clone(), values),
                Err(RootError::UnsupportedLayout { .. }) => BranchSnapshot::$jagged(
                    branch.name.clone(),
                    read_jagged_auto::<$ty>(tree, branch),
                ),
                Err(error) => panic!("failed to read `{}` as {type_name}: {error}", branch.name),
            }
        }};
    }

    match type_name.as_str() {
        "bool" => read_typed!(bool, Bool, VecBool),
        "i8" => read_typed!(i8, I8, VecI8),
        "u8" => read_typed!(u8, U8, VecU8),
        "i16" => read_typed!(i16, I16, VecI16),
        "u16" => read_typed!(u16, U16, VecU16),
        "i32" => read_typed!(i32, I32, VecI32),
        "u32" => read_typed!(u32, U32, VecU32),
        "i64" => read_typed!(i64, I64, VecI64),
        "u64" => read_typed!(u64, U64, VecU64),
        "f32" => read_typed!(f32, F32, VecF32),
        "f64" => read_typed!(f64, F64, VecF64),
        other => panic!("unsupported branch `{}` type `{other}`", branch.name),
    }
}

fn read_jagged_auto<T: Scalar>(tree: &Tree, branch: &BranchInfo) -> Vec<Vec<T>> {
    tree.read_jagged_auto::<T>(&branch.name)
        .unwrap_or_else(|error| panic!("failed to read `{}` as jagged: {error}", branch.name))
}

#[derive(Debug, Clone)]
enum BranchSnapshot {
    Bool(String, Vec<bool>),
    I8(String, Vec<i8>),
    U8(String, Vec<u8>),
    I16(String, Vec<i16>),
    U16(String, Vec<u16>),
    I32(String, Vec<i32>),
    U32(String, Vec<u32>),
    I64(String, Vec<i64>),
    U64(String, Vec<u64>),
    F32(String, Vec<f32>),
    F64(String, Vec<f64>),
    VecBool(String, Vec<Vec<bool>>),
    VecI8(String, Vec<Vec<i8>>),
    VecU8(String, Vec<Vec<u8>>),
    VecI16(String, Vec<Vec<i16>>),
    VecU16(String, Vec<Vec<u16>>),
    VecI32(String, Vec<Vec<i32>>),
    VecU32(String, Vec<Vec<u32>>),
    VecI64(String, Vec<Vec<i64>>),
    VecU64(String, Vec<Vec<u64>>),
    VecF32(String, Vec<Vec<f32>>),
    VecF64(String, Vec<Vec<f64>>),
}

impl BranchSnapshot {
    fn name(&self) -> &str {
        match self {
            Self::Bool(name, _)
            | Self::I8(name, _)
            | Self::U8(name, _)
            | Self::I16(name, _)
            | Self::U16(name, _)
            | Self::I32(name, _)
            | Self::U32(name, _)
            | Self::I64(name, _)
            | Self::U64(name, _)
            | Self::F32(name, _)
            | Self::F64(name, _)
            | Self::VecBool(name, _)
            | Self::VecI8(name, _)
            | Self::VecU8(name, _)
            | Self::VecI16(name, _)
            | Self::VecU16(name, _)
            | Self::VecI32(name, _)
            | Self::VecU32(name, _)
            | Self::VecI64(name, _)
            | Self::VecU64(name, _)
            | Self::VecF32(name, _)
            | Self::VecF64(name, _) => name,
        }
    }

    fn to_write_branch(&self) -> Branch {
        match self {
            Self::Bool(name, values) => Branch::bool(name, values.clone()),
            Self::I8(name, values) => Branch::i8(name, values.clone()),
            Self::U8(name, values) => Branch::u8(name, values.clone()),
            Self::I16(name, values) => Branch::i16(name, values.clone()),
            Self::U16(name, values) => Branch::u16(name, values.clone()),
            Self::I32(name, values) => Branch::i32(name, values.clone()),
            Self::U32(name, values) => Branch::u32(name, values.clone()),
            Self::I64(name, values) => Branch::i64(name, values.clone()),
            Self::U64(name, values) => Branch::u64(name, values.clone()),
            Self::F32(name, values) => Branch::f32(name, values.clone()),
            Self::F64(name, values) => Branch::f64(name, values.clone()),
            Self::VecBool(name, _) => {
                panic!("nano-rootio writer cannot write jagged bool branch `{name}`")
            }
            Self::VecI8(name, values) => Branch::vec_i8(name, values.clone()),
            Self::VecU8(name, values) => Branch::vec_u8(name, values.clone()),
            Self::VecI16(name, values) => Branch::vec_i16(name, values.clone()),
            Self::VecU16(name, values) => Branch::vec_u16(name, values.clone()),
            Self::VecI32(name, values) => Branch::vec_i32(name, values.clone()),
            Self::VecU32(name, values) => Branch::vec_u32(name, values.clone()),
            Self::VecI64(name, values) => Branch::vec_i64(name, values.clone()),
            Self::VecU64(name, values) => Branch::vec_u64(name, values.clone()),
            Self::VecF32(name, values) => Branch::vec_f32(name, values.clone()),
            Self::VecF64(name, values) => Branch::vec_f64(name, values.clone()),
        }
    }
}

fn write_snapshot(path: &Path, branches: &[BranchSnapshot]) {
    let root_branches = branches
        .iter()
        .map(BranchSnapshot::to_write_branch)
        .collect::<Vec<_>>();
    write_tree(path, "Events", &root_branches)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn assert_same_branch_data(reference: &[BranchSnapshot], candidate: &[BranchSnapshot]) {
    assert_eq!(reference.len(), candidate.len());
    for (reference, candidate) in reference.iter().zip(candidate) {
        assert_eq!(reference.name(), candidate.name());
        assert_same_branch(reference, candidate);
    }
}

fn assert_same_branch(reference: &BranchSnapshot, candidate: &BranchSnapshot) {
    match (reference, candidate) {
        (BranchSnapshot::Bool(_, reference), BranchSnapshot::Bool(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::I8(_, reference), BranchSnapshot::I8(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::U8(_, reference), BranchSnapshot::U8(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::I16(_, reference), BranchSnapshot::I16(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::U16(_, reference), BranchSnapshot::U16(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::I32(_, reference), BranchSnapshot::I32(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::U32(_, reference), BranchSnapshot::U32(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::I64(_, reference), BranchSnapshot::I64(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::U64(_, reference), BranchSnapshot::U64(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::F32(_, reference), BranchSnapshot::F32(_, candidate)) => {
            assert_same_floats(reference, candidate)
        }
        (BranchSnapshot::F64(_, reference), BranchSnapshot::F64(_, candidate)) => {
            assert_same_floats(reference, candidate)
        }
        (BranchSnapshot::VecBool(_, reference), BranchSnapshot::VecBool(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecI8(_, reference), BranchSnapshot::VecI8(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecU8(_, reference), BranchSnapshot::VecU8(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecI16(_, reference), BranchSnapshot::VecI16(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecU16(_, reference), BranchSnapshot::VecU16(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecI32(_, reference), BranchSnapshot::VecI32(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecU32(_, reference), BranchSnapshot::VecU32(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecI64(_, reference), BranchSnapshot::VecI64(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecU64(_, reference), BranchSnapshot::VecU64(_, candidate)) => {
            assert_eq!(reference, candidate)
        }
        (BranchSnapshot::VecF32(_, reference), BranchSnapshot::VecF32(_, candidate)) => {
            assert_same_jagged_floats(reference, candidate)
        }
        (BranchSnapshot::VecF64(_, reference), BranchSnapshot::VecF64(_, candidate)) => {
            assert_same_jagged_floats(reference, candidate)
        }
        (reference, candidate) => panic!(
            "branch `{}` changed representation: {reference:?} vs {candidate:?}",
            reference.name()
        ),
    }
}

trait SameFloat: Copy + std::fmt::Debug {
    fn same(self, other: Self) -> bool;
}

impl SameFloat for f32 {
    fn same(self, other: Self) -> bool {
        self == other || (self.is_nan() && other.is_nan())
    }
}

impl SameFloat for f64 {
    fn same(self, other: Self) -> bool {
        self == other || (self.is_nan() && other.is_nan())
    }
}

fn assert_same_floats<T: SameFloat>(reference: &[T], candidate: &[T]) {
    assert_eq!(reference.len(), candidate.len());
    for (index, (&reference, &candidate)) in reference.iter().zip(candidate).enumerate() {
        assert!(
            reference.same(candidate),
            "float mismatch at {index}: {reference:?} vs {candidate:?}"
        );
    }
}

fn assert_same_jagged_floats<T: SameFloat>(reference: &[Vec<T>], candidate: &[Vec<T>]) {
    assert_eq!(reference.len(), candidate.len());
    for (entry, (reference, candidate)) in reference.iter().zip(candidate).enumerate() {
        assert_eq!(
            reference.len(),
            candidate.len(),
            "row length at entry {entry}"
        );
        for (element, (&reference, &candidate)) in reference.iter().zip(candidate).enumerate() {
            assert!(
                reference.same(candidate),
                "float mismatch at entry {entry}[{element}]: {reference:?} vs {candidate:?}"
            );
        }
    }
}

fn perturb_scalar_f32(branches: &mut [BranchSnapshot], branch_name: &str) {
    let branch = branches
        .iter_mut()
        .find(|branch| branch.name() == branch_name)
        .unwrap_or_else(|| panic!("missing branch `{branch_name}`"));
    let BranchSnapshot::F32(_, values) = branch else {
        panic!("branch `{branch_name}` is not scalar f32: {branch:?}");
    };
    let value = values
        .iter_mut()
        .find(|value| value.is_finite())
        .unwrap_or_else(|| panic!("branch `{branch_name}` has no finite value to perturb"));
    *value += 10.0;
}

fn tight_compare_options() -> CompareOptions {
    CompareOptions {
        tree: "Events".to_string(),
        rtol: 0.0,
        atol: 0.0,
        max_mismatches: 5,
    }
}

fn assert_events_tree_parse_gap(relative: &str) {
    let path = repo_path(relative);
    let file = RootFile::open(&path).unwrap_or_else(|error| {
        panic!("failed to open {}: {error}", path.display());
    });
    assert!(
        file.objects()
            .iter()
            .any(|object| object.name() == "Events" && object.class() == "TTree"),
        "{} does not list an Events TTree",
        path.display()
    );
    let error = file
        .tree("Events")
        .expect_err("nano-rootio now parses this file; remove the ignore and enable full coverage");
    assert_eq!(
        format!("{error:?}"),
        "Parse { offset: 0, message: \"unexpected branch object class TLeafF\" }"
    );
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
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
            "nano-validate-frozen-{}-{timestamp}-{name}",
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
