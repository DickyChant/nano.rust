//! Adversarial mutation evidence matrix.
//!
//! | class | positive case | negative case | rejecting layer | expected rejection |
//! | --- | --- | --- | --- | --- |
//! | 1. Nonexistent/mistyped branch name | `good_muon.eta` validates against `nanov9` | `good_muon.etaa` is not in the catalogue | validator | `SpecError::MissingBranch` |
//! | 2. Era/NanoAOD-version mismatch | `Electron_mvaHZZIso` validates against `nanov12` | the same branch is absent from `nanov9` | validator | `SpecError::MissingBranch` |
//! | 3. Dropped/missing unit | `good_muon.pt > 30 GeV` validates | `good_muon.pt > 30` drops the momentum unit | validator | `SpecError::MissingUnit` |
//! | 4. Wrong-type branch access | `good_muon.pfRelIso04_all` validates as a numeric vector | `good_muon.looseId` is a bool vector used numerically | validator | `SpecError::WrongBranchType` |
//! | 5. Region/object referenced but not defined | `count(good_muon)` validates | `count(ghost_muon)` names no object | validator | `SpecError::UndefinedObject` |
//! | 6. Score-before-inference | `Muon_topscore` validates when produced by `[[model]]` | `Muon_topscore` is read with no producing model | validator | `SpecError::MissingBranch` |
//! | 7. Fill-before-weight | `fill` accepts `Weighted<R, S>`; see `nano-analysis` doctests | `fill` with `Ev<Raw>` | rustc | `compile_fail` doctest |
//! | 8. Missing systematic arm | complete `SystematicVisitor` impl; see `nano-analysis` doctests | visitor impl missing `jer_down` | rustc | `compile_fail` doctest |
//! | 9. Duplicate output name | distinct `[[outputs]]` names validate | duplicate ordinary output names | validator | `SpecError::InvalidExpression` |
//!
//! Branch existence, era catalogue membership, unit requirements, branch type,
//! undefined object, score production, and duplicate output names are
//! `nano_spec::validate` obligations. Typestate ordering and closed systematic
//! exhaustiveness are Rust type-system obligations and are checked by
//! `nano-analysis` doctests.

use nano_core::BranchType;
use nano_spec::{validate, AnalysisSpec, Catalogue, SpecError, Unit};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const NANOV12_CATALOGUE: &str = include_str!("../../../configs/branches/nanov12.yaml");

struct ValidatorCase {
    class: &'static str,
    positive_name: &'static str,
    positive_toml: &'static str,
    positive_catalogue: CatalogueVersion,
    negative_name: &'static str,
    negative_toml: &'static str,
    negative_catalogue: CatalogueVersion,
    expected: ExpectedError,
}

#[derive(Debug, Clone, Copy)]
enum CatalogueVersion {
    NanoV9,
    NanoV12,
}

impl CatalogueVersion {
    fn parse(self) -> Catalogue {
        match self {
            Self::NanoV9 => Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9")
                .expect("parse nanov9 catalogue"),
            Self::NanoV12 => Catalogue::from_nanoaod_yaml_str(NANOV12_CATALOGUE, "v12")
                .expect("parse nanov12 catalogue"),
        }
    }
}

enum ExpectedError {
    MissingBranch(&'static str),
    MissingUnit {
        expr: &'static str,
        expected: Unit,
    },
    WrongBranchType {
        branch: &'static str,
        actual: BranchType,
    },
    UndefinedObject(&'static str),
    InvalidExpressionContains(&'static str),
}

impl ExpectedError {
    fn matches(&self, error: &SpecError) -> bool {
        match (self, error) {
            (Self::MissingBranch(expected), SpecError::MissingBranch { branch, .. }) => {
                branch == expected
            }
            (
                Self::MissingUnit {
                    expr: expected_expr,
                    expected: expected_unit,
                },
                SpecError::MissingUnit { expr, expected, .. },
            ) => expr == expected_expr && expected == expected_unit,
            (
                Self::WrongBranchType {
                    branch: expected_branch,
                    actual: expected_actual,
                },
                SpecError::WrongBranchType { branch, actual, .. },
            ) => branch == expected_branch && actual == expected_actual,
            (Self::UndefinedObject(expected), SpecError::UndefinedObject { object, .. }) => {
                object == expected
            }
            (
                Self::InvalidExpressionContains(expected),
                SpecError::InvalidExpression { detail, .. },
            ) => detail.contains(expected),
            _ => false,
        }
    }
}

#[test]
fn adversarial_validator_reject_matrix() {
    let cases = [
        ValidatorCase {
            class: "nonexistent/mistyped branch name",
            positive_name: "known eta branch",
            positive_toml: MUON_ETA_SPEC,
            positive_catalogue: CatalogueVersion::NanoV9,
            negative_name: "mistyped eta branch",
            negative_toml: MUON_MISTYPED_BRANCH_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::MissingBranch("Muon_etaa"),
        },
        ValidatorCase {
            class: "era/NanoAOD-version mismatch",
            positive_name: "nanov12 electron HZZ MVA",
            positive_toml: ELECTRON_MVA_HZZ_SPEC,
            positive_catalogue: CatalogueVersion::NanoV12,
            negative_name: "nanov12-only branch under nanov9",
            negative_toml: ELECTRON_MVA_HZZ_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::MissingBranch("Electron_mvaHZZIso"),
        },
        ValidatorCase {
            class: "dropped/missing unit",
            positive_name: "pt cut with GeV",
            positive_toml: MUON_PT_WITH_UNIT_SPEC,
            positive_catalogue: CatalogueVersion::NanoV9,
            negative_name: "pt cut without GeV",
            negative_toml: MUON_PT_MISSING_UNIT_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::MissingUnit {
                expr: "good_muon.pt",
                expected: Unit::GeV,
            },
        },
        ValidatorCase {
            class: "wrong-type branch access",
            positive_name: "numeric isolation branch",
            positive_toml: MUON_NUMERIC_ISO_SPEC,
            positive_catalogue: CatalogueVersion::NanoV9,
            negative_name: "bool ID branch used numerically",
            negative_toml: MUON_BOOL_AS_NUMERIC_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::WrongBranchType {
                branch: "Muon_looseId",
                actual: BranchType::VecBool,
            },
        },
        ValidatorCase {
            class: "region/object referenced but not defined",
            positive_name: "defined object in region",
            positive_toml: REGION_DEFINED_OBJECT_SPEC,
            positive_catalogue: CatalogueVersion::NanoV9,
            negative_name: "undefined object in region",
            negative_toml: REGION_UNDEFINED_OBJECT_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::UndefinedObject("ghost_muon"),
        },
        ValidatorCase {
            class: "score-before-inference",
            positive_name: "model-produced score",
            positive_toml: MODEL_SCORE_PRODUCED_SPEC,
            positive_catalogue: CatalogueVersion::NanoV9,
            negative_name: "score read without model",
            negative_toml: MODEL_SCORE_UNPRODUCED_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::MissingBranch("Muon_topscore"),
        },
        ValidatorCase {
            class: "duplicate output name",
            positive_name: "distinct outputs",
            positive_toml: DISTINCT_OUTPUT_NAMES_SPEC,
            positive_catalogue: CatalogueVersion::NanoV9,
            negative_name: "duplicate outputs",
            negative_toml: DUPLICATE_OUTPUT_NAMES_SPEC,
            negative_catalogue: CatalogueVersion::NanoV9,
            expected: ExpectedError::InvalidExpressionContains(
                "duplicate output name `n_good_muon`",
            ),
        },
    ];

    for case in cases {
        let positive_spec = AnalysisSpec::from_toml_str(case.positive_toml)
            .unwrap_or_else(|error| panic!("{} positive parse failed: {error}", case.class));
        validate(&positive_spec, &case.positive_catalogue.parse()).unwrap_or_else(|errors| {
            panic!(
                "{} positive `{}` should validate, got: {errors:?}",
                case.class, case.positive_name
            )
        });

        let negative_spec = AnalysisSpec::from_toml_str(case.negative_toml)
            .unwrap_or_else(|error| panic!("{} negative parse failed: {error}", case.class));
        let errors = validate(&negative_spec, &case.negative_catalogue.parse()).unwrap_err();
        assert!(
            errors.iter().any(|error| case.expected.matches(error)),
            "{} negative `{}` should produce expected error, got: {errors:?}",
            case.class,
            case.negative_name
        );
    }
}

const MUON_ETA_SPEC: &str = r#"
[analysis]
name = "known_eta_branch"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["abs(eta) < 2.4"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const MUON_MISTYPED_BRANCH_SPEC: &str = r#"
[analysis]
name = "mistyped_eta_branch"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["abs(etaa) < 2.4"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const ELECTRON_MVA_HZZ_SPEC: &str = r#"
[analysis]
name = "electron_mva_hzz"
year = "Run2018"

[objects.hzz_electron]
source = "Electron"
cuts = ["mvaHZZIso > 0"]

[regions.signal]
require = ["count(hzz_electron) >= 1"]

[[outputs]]
name = "n_hzz_electron"
expr = "count(hzz_electron)"
"#;

const MUON_PT_WITH_UNIT_SPEC: &str = r#"
[analysis]
name = "pt_with_unit"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["pt > 30 GeV"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const MUON_PT_MISSING_UNIT_SPEC: &str = r#"
[analysis]
name = "pt_missing_unit"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["pt > 30"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const MUON_NUMERIC_ISO_SPEC: &str = r#"
[analysis]
name = "numeric_iso"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["pfRelIso04_all < 0.25"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const MUON_BOOL_AS_NUMERIC_SPEC: &str = r#"
[analysis]
name = "bool_as_numeric"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["looseId > 0"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const REGION_DEFINED_OBJECT_SPEC: &str = r#"
[analysis]
name = "defined_region_object"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const REGION_UNDEFINED_OBJECT_SPEC: &str = r#"
[analysis]
name = "undefined_region_object"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[regions.signal]
require = ["count(ghost_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#;

const MODEL_SCORE_PRODUCED_SPEC: &str = r#"
[analysis]
name = "score_produced"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["pt > 30 GeV"]

[[model]]
name = "muon_tagger"
inputs = ["Muon_pt", "Muon_eta", "Muon_phi"]
output = "Muon_topscore"
batch = "Muon"

[model.provider]
kind = "mock"

[regions.signal]
require = ["count(good_muon) >= 1", "leading(good_muon).topscore > 0.5"]

[[outputs]]
name = "lead_muon_topscore"
expr = "leading(good_muon).topscore"
"#;

const MODEL_SCORE_UNPRODUCED_SPEC: &str = r#"
[analysis]
name = "score_unproduced"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["pt > 30 GeV"]

[regions.signal]
require = ["count(good_muon) >= 1", "leading(good_muon).topscore > 0.5"]

[[outputs]]
name = "lead_muon_topscore"
expr = "leading(good_muon).topscore"
"#;

const DISTINCT_OUTPUT_NAMES_SPEC: &str = r#"
[analysis]
name = "distinct_output_names"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"

[[outputs]]
name = "lead_muon_pt"
expr = "leading(good_muon).pt"
"#;

const DUPLICATE_OUTPUT_NAMES_SPEC: &str = r#"
[analysis]
name = "duplicate_output_names"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"

[[outputs]]
name = "n_good_muon"
expr = "leading(good_muon).pt"
"#;
