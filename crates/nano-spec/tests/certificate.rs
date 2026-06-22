use std::collections::BTreeSet;

use nano_spec::certificate::{certify, PlanCertificate};
use nano_spec::kir;
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const MUON_TOML: &str = include_str!("../examples/muon.toml");
const MUON_ADL: &str = include_str!("../examples/muon.adl");
const MUON_HIST_NOMINAL_TOML: &str = include_str!("../examples/muon_hist_nominal.toml");
const MUON_TAGGER_TOML: &str = include_str!("../examples/muon_tagger.toml");

const PRESERVATION_EXAMPLES: &[(&str, &str)] = &[
    ("muon", include_str!("../examples/muon.toml")),
    ("dimuon", include_str!("../examples/dimuon.toml")),
    ("higgs4l", include_str!("../examples/higgs4l.toml")),
    (
        "muon_hist_weight_systematic",
        include_str!("../examples/muon_hist_weight_systematic.toml"),
    ),
    (
        "muon_hist_shape_correction",
        include_str!("../examples/muon_hist_shape_correction.toml"),
    ),
    ("zmumu_cr", include_str!("../examples/zmumu_cr.toml")),
    ("multijet_ht", include_str!("../examples/multijet_ht.toml")),
];

fn catalogue() -> Catalogue {
    Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue")
}

fn validated_toml(input: &str) -> nano_spec::ResolvedPlan {
    let spec = AnalysisSpec::from_toml_str(input).expect("parse TOML spec");
    validate(&spec, &catalogue()).expect("validate spec")
}

fn branch_names_from_plan(plan: &nano_spec::ResolvedPlan) -> BTreeSet<String> {
    plan.read_branches
        .specs()
        .iter()
        .map(|branch| branch.name.clone())
        .collect()
}

fn branch_names_from_certificate(certificate: &PlanCertificate) -> BTreeSet<String> {
    certificate
        .required_branches
        .iter()
        .map(|branch| branch.name.clone())
        .collect()
}

fn reads_branch_effects(certificate: &PlanCertificate) -> BTreeSet<String> {
    certificate
        .effects
        .iter()
        .filter(|effect| effect.kind == "ReadsBranch")
        .map(|effect| effect.value.clone())
        .collect()
}

#[test]
fn certificate_is_deterministic_with_fixed_hash_seed() {
    let plan = validated_toml(MUON_TOML);

    let first = certify(&plan);
    let second = certify(&plan);

    assert_eq!(first, second);
    assert_eq!(first.hash, second.hash);
    assert_eq!(first.hash.len(), 16);
    assert_eq!(
        serde_json::to_string(&first).expect("serialize first certificate"),
        serde_json::to_string(&second).expect("serialize second certificate")
    );
}

#[test]
fn certificate_preserves_read_branches_effects_and_kir_for_examples() {
    for (name, input) in PRESERVATION_EXAMPLES {
        let plan = validated_toml(input);
        let certificate = certify(&plan);
        let plan_branches = branch_names_from_plan(&plan);
        let kir = kir::lower_plan_to_kir(&plan)
            .unwrap_or_else(|error| panic!("lower {name} to KIR: {error}"));
        let kir_branches = kir
            .read_branches
            .iter()
            .map(|branch| branch.name.clone())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            branch_names_from_certificate(&certificate),
            plan_branches,
            "{name} certificate required branches drifted from ResolvedPlan"
        );
        assert_eq!(
            reads_branch_effects(&certificate),
            plan_branches,
            "{name} Core ReadsBranch effects drifted from ResolvedPlan"
        );
        assert_eq!(
            kir_branches, plan_branches,
            "{name} KIR read branches drifted from ResolvedPlan"
        );
    }
}

#[test]
fn certificate_hash_changes_when_meaning_changes() {
    let base = validated_toml(MUON_TOML);
    let base_hash = certify(&base).hash;

    let extra_branch_cut = MUON_TOML.replace("abs(eta) < 2.4", "abs(eta) < 2.4\", \"charge > 0");
    let extra_branch_hash = certify(&validated_toml(&extra_branch_cut)).hash;
    assert_ne!(base_hash, extra_branch_hash);

    let renamed_output = MUON_TOML.replace("name = \"lead_muon_pt\"", "name = \"lead_muon_eta\"");
    let renamed_output_hash = certify(&validated_toml(&renamed_output)).hash;
    assert_ne!(base_hash, renamed_output_hash);

    let nominal_hash = certify(&validated_toml(MUON_HIST_NOMINAL_TOML)).hash;
    let with_systematic = MUON_HIST_NOMINAL_TOML.replace(
        "[objects.good_muon]",
        "[[systematic]]\nname = \"muon_weight\"\nkind = \"weight\"\nup = 2.0\ndown = 0.5\n\n[objects.good_muon]",
    );
    let systematic_hash = certify(&validated_toml(&with_systematic)).hash;
    assert_ne!(nominal_hash, systematic_hash);
}

#[test]
fn certificate_is_identical_for_adl_and_equivalent_toml() {
    let catalogue = catalogue();
    let toml_spec = AnalysisSpec::from_toml_str(MUON_TOML).expect("parse TOML spec");
    let adl_spec = AnalysisSpec::from_adl_str(MUON_ADL).expect("parse ADL spec");
    let toml_plan = validate(&toml_spec, &catalogue).expect("validate TOML plan");
    let adl_plan = validate(&adl_spec, &catalogue).expect("validate ADL plan");

    assert_eq!(certify(&toml_plan), certify(&adl_plan));
}

#[test]
fn certificate_records_model_outputs_and_score_effects() {
    let certificate = certify(&validated_toml(MUON_TAGGER_TOML));

    assert_eq!(certificate.model_outputs.len(), 1);
    assert_eq!(certificate.model_outputs[0].model, "muon_tagger");
    assert_eq!(certificate.model_outputs[0].output, "Muon_topscore");
    assert!(certificate
        .effects
        .iter()
        .any(|effect| effect.kind == "ProducesScore" && effect.value == "Muon_topscore"));
    assert!(certificate.effects.iter().any(
        |effect| effect.kind == "RequiresModel" && effect.value == "muon_tagger:Muon_topscore"
    ));
}
