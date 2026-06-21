//! Golden ROOT skim comparison for nano.rust.
//!
//! The public entry point is [`compare_root_files`], which compares matching
//! branches in two TTrees and returns a structured [`ComparisonReport`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use nano_rootio::{BranchInfo, Error as RootError, RootFile, Scalar, Tree};
use serde::Serialize;

const DEFAULT_MAX_MISMATCHES: usize = 5;

pub type Result<T> = std::result::Result<T, ValidationError>;

#[derive(Debug)]
pub enum ValidationError {
    Root(RootError),
    UnsupportedBranchType { branch: String, types: Vec<String> },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root(error) => write!(f, "{error}"),
            Self::UnsupportedBranchType { branch, types } => {
                write!(
                    f,
                    "unsupported branch `{branch}` with type signature {types:?}"
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Root(error) => Some(error),
            Self::UnsupportedBranchType { .. } => None,
        }
    }
}

impl From<RootError> for ValidationError {
    fn from(error: RootError) -> Self {
        Self::Root(error)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareOptions {
    pub tree: String,
    pub rtol: f64,
    pub atol: f64,
    pub max_mismatches: usize,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            tree: "Events".to_string(),
            rtol: 1e-6,
            atol: 1e-6,
            max_mismatches: DEFAULT_MAX_MISMATCHES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComparisonReport {
    pub status: ComparisonStatus,
    pub reference: PathBuf,
    pub candidate: PathBuf,
    pub tree: String,
    pub tolerance: FloatTolerance,
    pub reference_entries: i64,
    pub candidate_entries: i64,
    pub compared_entries: i64,
    pub entry_count_match: bool,
    pub branches: Vec<BranchComparison>,
}

impl ComparisonReport {
    pub fn passed(&self) -> bool {
        self.status == ComparisonStatus::Pass
    }

    pub fn summary(&self) -> String {
        let present = self
            .branches
            .iter()
            .filter(|branch| branch.presence == BranchPresence::PresentInBoth)
            .count();
        let only_ref = self
            .branches
            .iter()
            .filter(|branch| branch.presence == BranchPresence::OnlyInReference)
            .count();
        let only_candidate = self
            .branches
            .iter()
            .filter(|branch| branch.presence == BranchPresence::OnlyInCandidate)
            .count();
        let mismatched = self
            .branches
            .iter()
            .filter(|branch| branch.n_mismatched > 0 || branch.error.is_some())
            .count();
        let total_value_mismatches = self
            .branches
            .iter()
            .map(|branch| branch.n_mismatched)
            .sum::<u64>();

        let mut lines = vec![format!(
            "{} compare {} vs {} tree={} entries ref={} candidate={} compared={}",
            match self.status {
                ComparisonStatus::Pass => "PASS",
                ComparisonStatus::Fail => "FAIL",
            },
            self.reference.display(),
            self.candidate.display(),
            self.tree,
            self.reference_entries,
            self.candidate_entries,
            self.compared_entries
        )];
        lines.push(format!(
            "branches: present_in_both={present} only_in_ref={only_ref} only_in_candidate={only_candidate} mismatched={mismatched} value_mismatches={total_value_mismatches}"
        ));
        lines.push(format!(
            "tolerance: abs_diff <= atol + rtol * abs(reference), rtol={} atol={}",
            self.tolerance.rtol, self.tolerance.atol
        ));

        for branch in &self.branches {
            match branch.presence {
                BranchPresence::PresentInBoth => {
                    if branch.n_mismatched > 0 || branch.error.is_some() {
                        lines.push(format!(
                            "  {}: compared={} mismatched={} max_abs_diff={} max_rel_diff={}{}",
                            branch.name,
                            branch.n_compared,
                            branch.n_mismatched,
                            format_optional_f64(branch.max_abs_diff),
                            format_optional_f64(branch.max_rel_diff),
                            branch
                                .error
                                .as_ref()
                                .map(|error| format!(" error={error}"))
                                .unwrap_or_default()
                        ));
                        for mismatch in &branch.first_mismatches {
                            let location = mismatch
                                .element
                                .map(|element| format!("{}[{element}]", mismatch.entry))
                                .unwrap_or_else(|| mismatch.entry.to_string());
                            lines.push(format!(
                                "    entry {location}: ref={} candidate={} abs_diff={} rel_diff={}",
                                mismatch.reference,
                                mismatch.candidate,
                                format_optional_f64(mismatch.abs_diff),
                                format_optional_f64(mismatch.rel_diff)
                            ));
                        }
                    }
                }
                BranchPresence::OnlyInReference => {
                    lines.push(format!("  {}: only_in_ref", branch.name))
                }
                BranchPresence::OnlyInCandidate => {
                    lines.push(format!("  {}: only_in_candidate", branch.name));
                }
            }
        }

        lines.join("\n")
    }
}

fn format_optional_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.6e}"))
        .unwrap_or_else(|| "n/a".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FloatTolerance {
    pub rtol: f64,
    pub atol: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BranchComparison {
    pub name: String,
    pub presence: BranchPresence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_kind: Option<ValueKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_types: Option<Vec<String>>,
    pub n_compared: u64,
    pub n_mismatched: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_abs_diff: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rel_diff: Option<f64>,
    pub first_mismatches: Vec<ValueMismatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BranchComparison {
    fn only(name: String, presence: BranchPresence, types: Option<Vec<String>>) -> Self {
        let (reference_types, candidate_types) = match presence {
            BranchPresence::OnlyInReference => (types, None),
            BranchPresence::OnlyInCandidate => (None, types),
            BranchPresence::PresentInBoth => (None, None),
        };
        Self {
            name,
            presence,
            value_kind: None,
            reference_types,
            candidate_types,
            n_compared: 0,
            n_mismatched: 0,
            max_abs_diff: None,
            max_rel_diff: None,
            first_mismatches: Vec::new(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchPresence {
    PresentInBoth,
    OnlyInReference,
    OnlyInCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    ScalarBool,
    ScalarI8,
    ScalarU8,
    ScalarI16,
    ScalarU16,
    ScalarI32,
    ScalarU32,
    ScalarI64,
    ScalarU64,
    ScalarF32,
    ScalarF64,
    JaggedBool,
    JaggedI8,
    JaggedU8,
    JaggedI16,
    JaggedU16,
    JaggedI32,
    JaggedU32,
    JaggedI64,
    JaggedU64,
    JaggedF32,
    JaggedF64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValueMismatch {
    pub entry: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<usize>,
    pub reference: String,
    pub candidate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abs_diff: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_diff: Option<f64>,
}

pub fn compare_root_files(
    reference: impl AsRef<Path>,
    candidate: impl AsRef<Path>,
    options: &CompareOptions,
) -> Result<ComparisonReport> {
    let reference_path = reference.as_ref();
    let candidate_path = candidate.as_ref();
    let reference_file = RootFile::open(reference_path)?;
    let candidate_file = RootFile::open(candidate_path)?;
    let reference_tree = reference_file.tree(&options.tree)?;
    let candidate_tree = candidate_file.tree(&options.tree)?;

    compare_trees(
        reference_path,
        candidate_path,
        &reference_tree,
        &candidate_tree,
        options,
    )
}

fn compare_trees(
    reference_path: &Path,
    candidate_path: &Path,
    reference_tree: &Tree,
    candidate_tree: &Tree,
    options: &CompareOptions,
) -> Result<ComparisonReport> {
    let reference_branches = branch_map(reference_tree);
    let candidate_branches = branch_map(candidate_tree);
    let names = reference_branches
        .keys()
        .chain(candidate_branches.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut comparisons = Vec::with_capacity(names.len());
    let tolerance = FloatTolerance {
        rtol: options.rtol,
        atol: options.atol,
    };

    for name in names {
        let comparison = match (reference_branches.get(&name), candidate_branches.get(&name)) {
            (Some(reference), Some(candidate)) => compare_branch(
                reference_tree,
                candidate_tree,
                reference,
                candidate,
                options,
            ),
            (Some(reference), None) => BranchComparison::only(
                name,
                BranchPresence::OnlyInReference,
                Some(reference.types.clone()),
            ),
            (None, Some(candidate)) => BranchComparison::only(
                name,
                BranchPresence::OnlyInCandidate,
                Some(candidate.types.clone()),
            ),
            (None, None) => unreachable!("name came from branch maps"),
        };
        comparisons.push(comparison);
    }

    let entry_count_match = reference_tree.entries() == candidate_tree.entries();
    let failed = !entry_count_match
        || comparisons.iter().any(|branch| {
            branch.presence != BranchPresence::PresentInBoth
                || branch.n_mismatched > 0
                || branch.error.is_some()
        });

    Ok(ComparisonReport {
        status: if failed {
            ComparisonStatus::Fail
        } else {
            ComparisonStatus::Pass
        },
        reference: reference_path.to_path_buf(),
        candidate: candidate_path.to_path_buf(),
        tree: options.tree.clone(),
        tolerance,
        reference_entries: reference_tree.entries(),
        candidate_entries: candidate_tree.entries(),
        compared_entries: reference_tree
            .entries()
            .min(candidate_tree.entries())
            .max(0),
        entry_count_match,
        branches: comparisons,
    })
}

fn branch_map(tree: &Tree) -> BTreeMap<String, BranchInfo> {
    tree.branches()
        .into_iter()
        .map(|branch| (branch.name.clone(), branch))
        .collect()
}

fn compare_branch(
    reference_tree: &Tree,
    candidate_tree: &Tree,
    reference: &BranchInfo,
    candidate: &BranchInfo,
    options: &CompareOptions,
) -> BranchComparison {
    let reference_type = match value_type(reference) {
        Ok(value_type) => value_type,
        Err(error) => {
            return branch_error(
                &reference.name,
                &reference.types,
                &candidate.types,
                error.to_string(),
            );
        }
    };
    let candidate_type = match value_type(candidate) {
        Ok(value_type) => value_type,
        Err(error) => {
            return branch_error(
                &reference.name,
                &reference.types,
                &candidate.types,
                error.to_string(),
            );
        }
    };
    if reference_type != candidate_type {
        return BranchComparison {
            name: reference.name.clone(),
            presence: BranchPresence::PresentInBoth,
            value_kind: None,
            reference_types: Some(reference.types.clone()),
            candidate_types: Some(candidate.types.clone()),
            n_compared: 0,
            n_mismatched: 1,
            max_abs_diff: None,
            max_rel_diff: None,
            first_mismatches: vec![ValueMismatch {
                entry: 0,
                element: None,
                reference: format!("{reference_type:?}"),
                candidate: format!("{candidate_type:?}"),
                abs_diff: None,
                rel_diff: None,
            }],
            error: Some("branch type mismatch".to_string()),
        };
    }

    let reference_values = match read_branch(reference_tree, &reference.name, reference_type) {
        Ok(values) => values,
        Err(error) => {
            return branch_error(
                &reference.name,
                &reference.types,
                &candidate.types,
                format!("failed to read reference branch: {error}"),
            );
        }
    };
    let candidate_values = match read_branch(candidate_tree, &candidate.name, candidate_type) {
        Ok(values) => values,
        Err(error) => {
            return branch_error(
                &reference.name,
                &reference.types,
                &candidate.types,
                format!("failed to read candidate branch: {error}"),
            );
        }
    };
    compare_values(
        &reference.name,
        reference_type,
        &reference.types,
        &candidate.types,
        reference_values,
        candidate_values,
        options,
    )
}

fn branch_error(
    name: &str,
    reference_types: &[String],
    candidate_types: &[String],
    error: String,
) -> BranchComparison {
    BranchComparison {
        name: name.to_string(),
        presence: BranchPresence::PresentInBoth,
        value_kind: None,
        reference_types: Some(reference_types.to_vec()),
        candidate_types: Some(candidate_types.to_vec()),
        n_compared: 0,
        n_mismatched: 1,
        max_abs_diff: None,
        max_rel_diff: None,
        first_mismatches: Vec::new(),
        error: Some(error),
    }
}

fn value_type(branch: &BranchInfo) -> Result<ValueType> {
    let [type_name] = branch.types.as_slice() else {
        return Err(ValidationError::UnsupportedBranchType {
            branch: branch.name.clone(),
            types: branch.types.clone(),
        });
    };
    ValueType::from_type_name(type_name).ok_or_else(|| ValidationError::UnsupportedBranchType {
        branch: branch.name.clone(),
        types: branch.types.clone(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueType {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl ValueType {
    fn from_type_name(type_name: &str) -> Option<Self> {
        match type_name {
            "bool" => Some(Self::Bool),
            "i8" => Some(Self::I8),
            "u8" => Some(Self::U8),
            "i16" => Some(Self::I16),
            "u16" => Some(Self::U16),
            "i32" => Some(Self::I32),
            "u32" => Some(Self::U32),
            "i64" => Some(Self::I64),
            "u64" => Some(Self::U64),
            "f32" => Some(Self::F32),
            "f64" => Some(Self::F64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum BranchValues {
    ScalarBool(Vec<bool>),
    ScalarI8(Vec<i8>),
    ScalarU8(Vec<u8>),
    ScalarI16(Vec<i16>),
    ScalarU16(Vec<u16>),
    ScalarI32(Vec<i32>),
    ScalarU32(Vec<u32>),
    ScalarI64(Vec<i64>),
    ScalarU64(Vec<u64>),
    ScalarF32(Vec<f32>),
    ScalarF64(Vec<f64>),
    JaggedBool(Vec<Vec<bool>>),
    JaggedI8(Vec<Vec<i8>>),
    JaggedU8(Vec<Vec<u8>>),
    JaggedI16(Vec<Vec<i16>>),
    JaggedU16(Vec<Vec<u16>>),
    JaggedI32(Vec<Vec<i32>>),
    JaggedU32(Vec<Vec<u32>>),
    JaggedI64(Vec<Vec<i64>>),
    JaggedU64(Vec<Vec<u64>>),
    JaggedF32(Vec<Vec<f32>>),
    JaggedF64(Vec<Vec<f64>>),
}

impl BranchValues {
    fn kind(&self) -> ValueKind {
        match self {
            Self::ScalarBool(_) => ValueKind::ScalarBool,
            Self::ScalarI8(_) => ValueKind::ScalarI8,
            Self::ScalarU8(_) => ValueKind::ScalarU8,
            Self::ScalarI16(_) => ValueKind::ScalarI16,
            Self::ScalarU16(_) => ValueKind::ScalarU16,
            Self::ScalarI32(_) => ValueKind::ScalarI32,
            Self::ScalarU32(_) => ValueKind::ScalarU32,
            Self::ScalarI64(_) => ValueKind::ScalarI64,
            Self::ScalarU64(_) => ValueKind::ScalarU64,
            Self::ScalarF32(_) => ValueKind::ScalarF32,
            Self::ScalarF64(_) => ValueKind::ScalarF64,
            Self::JaggedBool(_) => ValueKind::JaggedBool,
            Self::JaggedI8(_) => ValueKind::JaggedI8,
            Self::JaggedU8(_) => ValueKind::JaggedU8,
            Self::JaggedI16(_) => ValueKind::JaggedI16,
            Self::JaggedU16(_) => ValueKind::JaggedU16,
            Self::JaggedI32(_) => ValueKind::JaggedI32,
            Self::JaggedU32(_) => ValueKind::JaggedU32,
            Self::JaggedI64(_) => ValueKind::JaggedI64,
            Self::JaggedU64(_) => ValueKind::JaggedU64,
            Self::JaggedF32(_) => ValueKind::JaggedF32,
            Self::JaggedF64(_) => ValueKind::JaggedF64,
        }
    }
}

fn read_branch(tree: &Tree, name: &str, value_type: ValueType) -> Result<BranchValues> {
    macro_rules! read_typed {
        ($ty:ty, $scalar:ident, $jagged:ident) => {
            match tree.read_scalar::<$ty>(name) {
                Ok(values) => Ok(BranchValues::$scalar(values)),
                Err(RootError::UnsupportedLayout { .. }) => tree
                    .read_jagged_auto::<$ty>(name)
                    .map(BranchValues::$jagged)
                    .map_err(ValidationError::from),
                Err(error) => Err(ValidationError::from(error)),
            }
        };
    }

    match value_type {
        ValueType::Bool => read_typed!(bool, ScalarBool, JaggedBool),
        ValueType::I8 => read_typed!(i8, ScalarI8, JaggedI8),
        ValueType::U8 => read_typed!(u8, ScalarU8, JaggedU8),
        ValueType::I16 => read_typed!(i16, ScalarI16, JaggedI16),
        ValueType::U16 => read_typed!(u16, ScalarU16, JaggedU16),
        ValueType::I32 => read_typed!(i32, ScalarI32, JaggedI32),
        ValueType::U32 => read_typed!(u32, ScalarU32, JaggedU32),
        ValueType::I64 => read_typed!(i64, ScalarI64, JaggedI64),
        ValueType::U64 => read_typed!(u64, ScalarU64, JaggedU64),
        ValueType::F32 => read_typed!(f32, ScalarF32, JaggedF32),
        ValueType::F64 => read_typed!(f64, ScalarF64, JaggedF64),
    }
}

fn compare_values(
    name: &str,
    value_type: ValueType,
    reference_types: &[String],
    candidate_types: &[String],
    reference: BranchValues,
    candidate: BranchValues,
    options: &CompareOptions,
) -> BranchComparison {
    let mut accumulator = Accumulator::new(options);
    let error = match (reference, candidate) {
        (BranchValues::ScalarBool(reference), BranchValues::ScalarBool(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarI8(reference), BranchValues::ScalarI8(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarU8(reference), BranchValues::ScalarU8(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarI16(reference), BranchValues::ScalarI16(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarU16(reference), BranchValues::ScalarU16(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarI32(reference), BranchValues::ScalarI32(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarU32(reference), BranchValues::ScalarU32(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarI64(reference), BranchValues::ScalarI64(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarU64(reference), BranchValues::ScalarU64(candidate)) => {
            compare_exact_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarF32(reference), BranchValues::ScalarF32(candidate)) => {
            compare_float_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::ScalarF64(reference), BranchValues::ScalarF64(candidate)) => {
            compare_float_slice(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedBool(reference), BranchValues::JaggedBool(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedI8(reference), BranchValues::JaggedI8(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedU8(reference), BranchValues::JaggedU8(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedI16(reference), BranchValues::JaggedI16(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedU16(reference), BranchValues::JaggedU16(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedI32(reference), BranchValues::JaggedI32(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedU32(reference), BranchValues::JaggedU32(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedI64(reference), BranchValues::JaggedI64(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedU64(reference), BranchValues::JaggedU64(candidate)) => {
            compare_exact_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedF32(reference), BranchValues::JaggedF32(candidate)) => {
            compare_float_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (BranchValues::JaggedF64(reference), BranchValues::JaggedF64(candidate)) => {
            compare_float_jagged(&mut accumulator, &reference, &candidate);
            None
        }
        (reference, candidate) => Some(format!(
            "branch layout mismatch: reference={:?} candidate={:?}",
            reference.kind(),
            candidate.kind()
        )),
    };

    let value_kind = if error.is_none() {
        Some(accumulator.kind)
    } else {
        Some(kind_for(value_type, false))
    };

    BranchComparison {
        name: name.to_string(),
        presence: BranchPresence::PresentInBoth,
        value_kind,
        reference_types: Some(reference_types.to_vec()),
        candidate_types: Some(candidate_types.to_vec()),
        n_compared: accumulator.n_compared,
        n_mismatched: accumulator.n_mismatched,
        max_abs_diff: accumulator.max_abs_diff,
        max_rel_diff: accumulator.max_rel_diff,
        first_mismatches: accumulator.first_mismatches,
        error,
    }
}

fn kind_for(value_type: ValueType, jagged: bool) -> ValueKind {
    match (value_type, jagged) {
        (ValueType::Bool, false) => ValueKind::ScalarBool,
        (ValueType::I8, false) => ValueKind::ScalarI8,
        (ValueType::U8, false) => ValueKind::ScalarU8,
        (ValueType::I16, false) => ValueKind::ScalarI16,
        (ValueType::U16, false) => ValueKind::ScalarU16,
        (ValueType::I32, false) => ValueKind::ScalarI32,
        (ValueType::U32, false) => ValueKind::ScalarU32,
        (ValueType::I64, false) => ValueKind::ScalarI64,
        (ValueType::U64, false) => ValueKind::ScalarU64,
        (ValueType::F32, false) => ValueKind::ScalarF32,
        (ValueType::F64, false) => ValueKind::ScalarF64,
        (ValueType::Bool, true) => ValueKind::JaggedBool,
        (ValueType::I8, true) => ValueKind::JaggedI8,
        (ValueType::U8, true) => ValueKind::JaggedU8,
        (ValueType::I16, true) => ValueKind::JaggedI16,
        (ValueType::U16, true) => ValueKind::JaggedU16,
        (ValueType::I32, true) => ValueKind::JaggedI32,
        (ValueType::U32, true) => ValueKind::JaggedU32,
        (ValueType::I64, true) => ValueKind::JaggedI64,
        (ValueType::U64, true) => ValueKind::JaggedU64,
        (ValueType::F32, true) => ValueKind::JaggedF32,
        (ValueType::F64, true) => ValueKind::JaggedF64,
    }
}

struct Accumulator<'a> {
    options: &'a CompareOptions,
    kind: ValueKind,
    n_compared: u64,
    n_mismatched: u64,
    max_abs_diff: Option<f64>,
    max_rel_diff: Option<f64>,
    first_mismatches: Vec<ValueMismatch>,
}

impl<'a> Accumulator<'a> {
    fn new(options: &'a CompareOptions) -> Self {
        Self {
            options,
            kind: ValueKind::ScalarBool,
            n_compared: 0,
            n_mismatched: 0,
            max_abs_diff: None,
            max_rel_diff: None,
            first_mismatches: Vec::new(),
        }
    }

    fn set_kind(&mut self, kind: ValueKind) {
        self.kind = kind;
    }

    fn compared(&mut self) {
        self.n_compared += 1;
    }

    fn mismatch(
        &mut self,
        entry: i64,
        element: Option<usize>,
        reference: String,
        candidate: String,
        diff: Option<FloatDiff>,
    ) {
        self.n_mismatched += 1;
        if let Some(diff) = diff {
            self.max_abs_diff = Some(
                self.max_abs_diff
                    .map(|current| current.max(diff.abs))
                    .unwrap_or(diff.abs),
            );
            self.max_rel_diff = Some(
                self.max_rel_diff
                    .map(|current| current.max(diff.rel))
                    .unwrap_or(diff.rel),
            );
        }
        if self.first_mismatches.len() < self.options.max_mismatches {
            self.first_mismatches.push(ValueMismatch {
                entry,
                element,
                reference,
                candidate,
                abs_diff: diff.map(|diff| diff.abs),
                rel_diff: diff.map(|diff| diff.rel),
            });
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FloatDiff {
    abs: f64,
    rel: f64,
}

fn compare_exact_slice<T>(accumulator: &mut Accumulator<'_>, reference: &[T], candidate: &[T])
where
    T: fmt::Debug + PartialEq + 'static,
{
    let kind = exact_kind::<T>(false);
    accumulator.set_kind(kind);
    let len = reference.len().min(candidate.len());
    for index in 0..len {
        accumulator.compared();
        if reference[index] != candidate[index] {
            accumulator.mismatch(
                index as i64,
                None,
                format!("{:?}", reference[index]),
                format!("{:?}", candidate[index]),
                None,
            );
        }
    }
    compare_len(accumulator, len, reference.len(), candidate.len(), None);
}

fn compare_exact_jagged<T>(
    accumulator: &mut Accumulator<'_>,
    reference: &[Vec<T>],
    candidate: &[Vec<T>],
) where
    T: fmt::Debug + PartialEq + 'static,
{
    accumulator.set_kind(exact_kind::<T>(true));
    let rows = reference.len().min(candidate.len());
    for entry in 0..rows {
        let reference_row = &reference[entry];
        let candidate_row = &candidate[entry];
        let len = reference_row.len().min(candidate_row.len());
        for element in 0..len {
            accumulator.compared();
            if reference_row[element] != candidate_row[element] {
                accumulator.mismatch(
                    entry as i64,
                    Some(element),
                    format!("{:?}", reference_row[element]),
                    format!("{:?}", candidate_row[element]),
                    None,
                );
            }
        }
        compare_len(
            accumulator,
            len,
            reference_row.len(),
            candidate_row.len(),
            Some(entry as i64),
        );
    }
    compare_len(accumulator, rows, reference.len(), candidate.len(), None);
}

fn exact_kind<T: 'static>(jagged: bool) -> ValueKind {
    if std::any::TypeId::of::<T>() == std::any::TypeId::of::<bool>() {
        kind_for(ValueType::Bool, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<i8>() {
        kind_for(ValueType::I8, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<u8>() {
        kind_for(ValueType::U8, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<i16>() {
        kind_for(ValueType::I16, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<u16>() {
        kind_for(ValueType::U16, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<i32>() {
        kind_for(ValueType::I32, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<u32>() {
        kind_for(ValueType::U32, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<i64>() {
        kind_for(ValueType::I64, jagged)
    } else if std::any::TypeId::of::<T>() == std::any::TypeId::of::<u64>() {
        kind_for(ValueType::U64, jagged)
    } else {
        unreachable!("exact comparison called for unsupported type")
    }
}

fn compare_float_slice<T>(accumulator: &mut Accumulator<'_>, reference: &[T], candidate: &[T])
where
    T: FloatValue,
{
    accumulator.set_kind(T::kind(false));
    let len = reference.len().min(candidate.len());
    for index in 0..len {
        accumulator.compared();
        compare_float_value(
            accumulator,
            index as i64,
            None,
            reference[index],
            candidate[index],
        );
    }
    compare_len(accumulator, len, reference.len(), candidate.len(), None);
}

fn compare_float_jagged<T>(
    accumulator: &mut Accumulator<'_>,
    reference: &[Vec<T>],
    candidate: &[Vec<T>],
) where
    T: FloatValue,
{
    accumulator.set_kind(T::kind(true));
    let rows = reference.len().min(candidate.len());
    for entry in 0..rows {
        let reference_row = &reference[entry];
        let candidate_row = &candidate[entry];
        let len = reference_row.len().min(candidate_row.len());
        for element in 0..len {
            accumulator.compared();
            compare_float_value(
                accumulator,
                entry as i64,
                Some(element),
                reference_row[element],
                candidate_row[element],
            );
        }
        compare_len(
            accumulator,
            len,
            reference_row.len(),
            candidate_row.len(),
            Some(entry as i64),
        );
    }
    compare_len(accumulator, rows, reference.len(), candidate.len(), None);
}

fn compare_float_value<T: FloatValue>(
    accumulator: &mut Accumulator<'_>,
    entry: i64,
    element: Option<usize>,
    reference: T,
    candidate: T,
) {
    let reference = reference.to_f64();
    let candidate = candidate.to_f64();
    if floats_match(reference, candidate, accumulator.options) {
        let diff = float_diff(reference, candidate);
        accumulator.max_abs_diff = Some(
            accumulator
                .max_abs_diff
                .map(|current| current.max(diff.abs))
                .unwrap_or(diff.abs),
        );
        accumulator.max_rel_diff = Some(
            accumulator
                .max_rel_diff
                .map(|current| current.max(diff.rel))
                .unwrap_or(diff.rel),
        );
        return;
    }
    let diff = float_diff(reference, candidate);
    accumulator.mismatch(
        entry,
        element,
        format_float(reference),
        format_float(candidate),
        Some(diff),
    );
}

fn compare_len(
    accumulator: &mut Accumulator<'_>,
    compared_len: usize,
    reference_len: usize,
    candidate_len: usize,
    entry: Option<i64>,
) {
    if reference_len == candidate_len {
        return;
    }
    let missing = reference_len.abs_diff(candidate_len);
    for offset in 0..missing {
        let base_entry = entry.unwrap_or((compared_len + offset) as i64);
        let element = entry.map(|_| compared_len + offset);
        accumulator.mismatch(
            base_entry,
            element,
            if compared_len + offset < reference_len {
                "present".to_string()
            } else {
                "missing".to_string()
            },
            if compared_len + offset < candidate_len {
                "present".to_string()
            } else {
                "missing".to_string()
            },
            None,
        );
    }
}

fn floats_match(reference: f64, candidate: f64, options: &CompareOptions) -> bool {
    if reference == candidate {
        return true;
    }
    if reference.is_nan() && candidate.is_nan() {
        return true;
    }
    if !reference.is_finite() || !candidate.is_finite() {
        return false;
    }
    let abs = (reference - candidate).abs();
    abs <= options.atol + options.rtol * reference.abs()
}

fn float_diff(reference: f64, candidate: f64) -> FloatDiff {
    let abs = (reference - candidate).abs();
    let rel = if reference == 0.0 {
        if abs == 0.0 {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        abs / reference.abs()
    };
    FloatDiff { abs, rel }
}

fn format_float(value: f64) -> String {
    format!("{value:.9e}")
}

trait FloatValue: Copy + Scalar {
    fn to_f64(self) -> f64;
    fn kind(jagged: bool) -> ValueKind;
}

impl FloatValue for f32 {
    fn to_f64(self) -> f64 {
        self as f64
    }

    fn kind(jagged: bool) -> ValueKind {
        kind_for(ValueType::F32, jagged)
    }
}

impl FloatValue for f64 {
    fn to_f64(self) -> f64 {
        self
    }

    fn kind(jagged: bool) -> ValueKind {
        kind_for(ValueType::F64, jagged)
    }
}
