use nano_spec::{
    AnalysisSpec, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr, HistogramDef,
    ObjectCandidateDef, ObjectDef, ObjectPairDef, OutputDef, PairConstraint, PairSelection,
    Quantity, RegionDef, Requirement, ShapeCorrectionDef, SystematicDef, Unit, WeightDef,
    WeightSystematicDef, Year,
};

pub const FUZZ_SEED: u64 = 0x4e41_4e4f_5f44_4946;
pub const FUZZ_SPEC_COUNT: usize = 400;

#[derive(Debug, Clone)]
pub struct GeneratedSpec {
    pub index: usize,
    pub spec: AnalysisSpec,
    pub has_histogram: bool,
    pub has_weight_systematic: bool,
    pub has_shape_correction: bool,
    pub has_derived_object: bool,
    pub has_candidate_object: bool,
}

#[derive(Debug, Clone, Copy)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn usize(&mut self, upper: usize) -> usize {
        (self.next_u64() % upper as u64) as usize
    }

    fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    fn f64(&mut self, low: f64, high: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / ((1_u64 << 53) as f64);
        low + (high - low) * unit
    }
}

#[derive(Debug, Clone, Copy)]
struct ObjectTemplate {
    name: &'static str,
    source: &'static str,
    cut_attrs: &'static [CutAttr],
}

#[derive(Debug, Clone, Copy)]
struct CutAttr {
    name: &'static str,
    unit: Unit,
    form: CutForm,
}

#[derive(Debug, Clone, Copy)]
enum CutForm {
    Greater { low: f64, high: f64 },
    AbsLess { low: f64, high: f64 },
    Less { low: f64, high: f64 },
}

const MUON_CUTS: &[CutAttr] = &[
    CutAttr {
        name: "pt",
        unit: Unit::GeV,
        form: CutForm::Greater {
            low: 3.0,
            high: 55.0,
        },
    },
    CutAttr {
        name: "eta",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 1.0,
            high: 2.5,
        },
    },
    CutAttr {
        name: "phi",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 1.0,
            high: 3.2,
        },
    },
    CutAttr {
        name: "mass",
        unit: Unit::GeV,
        form: CutForm::Greater {
            low: 0.0,
            high: 0.2,
        },
    },
    CutAttr {
        name: "dxy",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 0.01,
            high: 0.35,
        },
    },
    CutAttr {
        name: "dz",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 0.02,
            high: 0.7,
        },
    },
    CutAttr {
        name: "pfRelIso03_all",
        unit: Unit::Dimensionless,
        form: CutForm::Less {
            low: 0.05,
            high: 0.7,
        },
    },
];

const ELECTRON_CUTS: &[CutAttr] = &[
    CutAttr {
        name: "pt",
        unit: Unit::GeV,
        form: CutForm::Greater {
            low: 5.0,
            high: 60.0,
        },
    },
    CutAttr {
        name: "eta",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 1.0,
            high: 2.5,
        },
    },
    CutAttr {
        name: "phi",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 1.0,
            high: 3.2,
        },
    },
    CutAttr {
        name: "mass",
        unit: Unit::GeV,
        form: CutForm::Greater {
            low: 0.0,
            high: 0.2,
        },
    },
    CutAttr {
        name: "dxy",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 0.01,
            high: 0.35,
        },
    },
    CutAttr {
        name: "dz",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 0.02,
            high: 0.7,
        },
    },
    CutAttr {
        name: "pfRelIso03_all",
        unit: Unit::Dimensionless,
        form: CutForm::Less {
            low: 0.05,
            high: 0.7,
        },
    },
];

const JET_CUTS: &[CutAttr] = &[
    CutAttr {
        name: "pt",
        unit: Unit::GeV,
        form: CutForm::Greater {
            low: 15.0,
            high: 140.0,
        },
    },
    CutAttr {
        name: "eta",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 1.0,
            high: 4.7,
        },
    },
    CutAttr {
        name: "phi",
        unit: Unit::Dimensionless,
        form: CutForm::AbsLess {
            low: 1.0,
            high: 3.2,
        },
    },
    CutAttr {
        name: "mass",
        unit: Unit::GeV,
        form: CutForm::Greater {
            low: 0.0,
            high: 50.0,
        },
    },
    CutAttr {
        name: "btagDeepFlavB",
        unit: Unit::Dimensionless,
        form: CutForm::Greater {
            low: 0.01,
            high: 0.8,
        },
    },
    CutAttr {
        name: "area",
        unit: Unit::Dimensionless,
        form: CutForm::Less {
            low: 0.3,
            high: 1.2,
        },
    },
];

const OBJECTS: &[ObjectTemplate] = &[
    ObjectTemplate {
        name: "good_muon",
        source: "Muon",
        cut_attrs: MUON_CUTS,
    },
    ObjectTemplate {
        name: "good_electron",
        source: "Electron",
        cut_attrs: ELECTRON_CUTS,
    },
    ObjectTemplate {
        name: "good_jet",
        source: "Jet",
        cut_attrs: JET_CUTS,
    },
];

pub fn generated_specs() -> Vec<GeneratedSpec> {
    let mut rng = SplitMix64::new(FUZZ_SEED);
    (0..FUZZ_SPEC_COUNT)
        .map(|index| generated_spec(index, &mut rng))
        .collect()
}

fn generated_spec(index: usize, rng: &mut SplitMix64) -> GeneratedSpec {
    let objects = OBJECTS
        .iter()
        .map(|object| ObjectDef {
            name: object.name.to_string(),
            source: object.source.to_string(),
            cuts: object_cuts(object, rng),
        })
        .collect::<Vec<_>>();

    let derived_case = rng.usize(5);
    let derived_objects = derived_objects_for_case(derived_case, rng);
    let has_derived_object = !derived_objects.is_empty();
    let has_candidate_object = derived_objects
        .iter()
        .any(|object| matches!(object.source, DerivedSource::Candidate(_)));

    let mut first_region_requirements = vec![
        count_requirement("good_muon", CmpOp::Ge, 1.0),
        count_requirement("good_electron", CmpOp::Ge, 1.0),
        count_requirement("good_jet", CmpOp::Ge, 1.0),
    ];
    if rng.bool() {
        first_region_requirements.push(count_requirement(
            OBJECTS[rng.usize(OBJECTS.len())].name,
            CmpOp::Le,
            rng.f64(2.0, 5.0).floor(),
        ));
    }
    if let Some(requirement) = derived_requirement(&derived_objects, rng) {
        first_region_requirements.push(requirement);
    }

    let mut regions = vec![RegionDef {
        name: "baseline".to_string(),
        require: first_region_requirements,
    }];
    let extra_regions = 2 + rng.usize(2);
    for region_index in 1..extra_regions {
        let object = OBJECTS[rng.usize(OBJECTS.len())].name;
        let mut require = vec![count_requirement(
            object,
            if rng.bool() { CmpOp::Ge } else { CmpOp::Le },
            if rng.bool() {
                rng.f64(1.0, 4.0).floor()
            } else {
                rng.f64(2.0, 5.0).floor()
            },
        )];
        if rng.bool() {
            require.push(collection_predicate_requirement(object, rng));
        }
        if let Some(requirement) = derived_requirement(&derived_objects, rng) {
            require.push(requirement);
        }
        regions.push(RegionDef {
            name: format!("region_{region_index}"),
            require,
        });
    }

    let has_histogram = rng.usize(3) != 0;
    let has_shape_correction = has_histogram && !has_weight_systematic_candidate(rng) && rng.bool();
    let has_weight_systematic = has_histogram && !has_shape_correction && rng.bool();
    let histogram_expr = histogram_expr(&derived_objects, rng);
    let histograms = if has_histogram {
        vec![HistogramDef {
            name: "fuzz_hist".to_string(),
            expr: histogram_expr,
            bins: 4 + rng.usize(9),
            range: [0.0, 220.0],
        }]
    } else {
        Vec::new()
    };
    let systematics = if has_weight_systematic {
        vec![
            SystematicDef::Nominal,
            SystematicDef::Weight(WeightSystematicDef {
                name: format!("fuzz_weight_{index:03}"),
                up: round(rng.f64(1.1, 2.5)),
                down: round(rng.f64(0.25, 0.9)),
            }),
        ]
    } else {
        vec![SystematicDef::Nominal]
    };
    let weight = if has_histogram && rng.bool() {
        WeightDef {
            nominal: vec![round(rng.f64(0.4, 1.8))],
        }
    } else {
        WeightDef::default()
    };
    let shape_corrections = if has_shape_correction {
        vec![ShapeCorrectionDef {
            name: format!("fuzz_shape_{index:03}"),
            collection: OBJECTS[rng.usize(OBJECTS.len())].name.to_string(),
            attr: "pt".to_string(),
            up: round(rng.f64(1.02, 1.18)),
            down: round(rng.f64(0.82, 0.98)),
        }]
    } else {
        Vec::new()
    };

    GeneratedSpec {
        index,
        spec: AnalysisSpec {
            name: format!("fuzz_diff_{index:03}"),
            year: Year::Run2018,
            objects,
            derived_objects,
            models: Vec::new(),
            regions,
            outputs: outputs_for_case(derived_case),
            histograms,
            weight,
            systematics,
            shape_corrections,
            channels: Vec::new(),
        },
        has_histogram,
        has_weight_systematic,
        has_shape_correction,
        has_derived_object,
        has_candidate_object,
    }
}

fn has_weight_systematic_candidate(rng: &mut SplitMix64) -> bool {
    rng.usize(4) == 0
}

fn derived_objects_for_case(case: usize, rng: &mut SplitMix64) -> Vec<DerivedObjectDef> {
    match case {
        0 => Vec::new(),
        1 => vec![pair_object(
            "dilepton",
            if rng.bool() {
                "good_muon"
            } else {
                "good_electron"
            },
            if rng.bool() {
                PairSelection::LeadingPt
            } else {
                PairSelection::NearestMass {
                    target: q(91.2, Unit::GeV),
                }
            },
            vec![PairConstraint::OppositeCharge],
            maybe_pair_filters(rng),
            Vec::new(),
        )],
        2 => vec![pair_object(
            "dijet",
            "good_jet",
            PairSelection::LeadingPt,
            Vec::new(),
            maybe_pair_filters(rng),
            Vec::new(),
        )],
        3 => {
            let object = if rng.bool() {
                "good_muon"
            } else {
                "good_electron"
            };
            vec![
                pair_object(
                    "z1",
                    object,
                    PairSelection::NearestMassTruncated {
                        target: q(91.2, Unit::GeV),
                    },
                    vec![PairConstraint::OppositeCharge],
                    maybe_pair_filters(rng),
                    Vec::new(),
                ),
                pair_object(
                    "z2",
                    object,
                    PairSelection::LeadingPt,
                    vec![PairConstraint::OppositeCharge],
                    maybe_pair_filters(rng),
                    vec!["z1".to_string()],
                ),
                candidate_object("h4", vec!["z1", "z2"], maybe_candidate_filters(rng)),
            ]
        }
        _ => vec![candidate_object(
            "emu",
            vec!["good_muon", "good_electron"],
            maybe_candidate_filters(rng),
        )],
    }
}

fn pair_object(
    name: &str,
    object: &str,
    selection: PairSelection,
    constraints: Vec<PairConstraint>,
    filters: Vec<Cut>,
    exclude: Vec<String>,
) -> DerivedObjectDef {
    DerivedObjectDef {
        name: name.to_string(),
        source: DerivedSource::Pair(ObjectPairDef {
            object: object.to_string(),
            constraints,
            filters,
            selection,
            exclude,
        }),
    }
}

fn candidate_object(name: &str, items: Vec<&str>, filters: Vec<Cut>) -> DerivedObjectDef {
    DerivedObjectDef {
        name: name.to_string(),
        source: DerivedSource::Candidate(ObjectCandidateDef {
            items: items.into_iter().map(str::to_string).collect(),
            filters,
        }),
    }
}

fn maybe_pair_filters(rng: &mut SplitMix64) -> Vec<Cut> {
    let mut filters = Vec::new();
    if rng.bool() {
        filters.push(Cut {
            lhs: Expr::PairDeltaR,
            op: CmpOp::Ge,
            rhs: q(round(rng.f64(0.02, 0.5)), Unit::Dimensionless),
        });
    }
    if rng.bool() {
        filters.push(Cut {
            lhs: Expr::PairSubleadingPt,
            op: CmpOp::Gt,
            rhs: q(round(rng.f64(5.0, 25.0)), Unit::GeV),
        });
    }
    filters
}

fn maybe_candidate_filters(rng: &mut SplitMix64) -> Vec<Cut> {
    let mut filters = Vec::new();
    if rng.bool() {
        filters.push(Cut {
            lhs: Expr::CandidateMinDeltaR,
            op: CmpOp::Ge,
            rhs: q(round(rng.f64(0.02, 0.5)), Unit::Dimensionless),
        });
    }
    if rng.bool() {
        filters.push(Cut {
            lhs: Expr::CandidateSubleadingPt,
            op: CmpOp::Gt,
            rhs: q(round(rng.f64(5.0, 30.0)), Unit::GeV),
        });
    }
    filters
}

fn derived_requirement(
    derived_objects: &[DerivedObjectDef],
    rng: &mut SplitMix64,
) -> Option<Requirement> {
    let derived = derived_objects.get(rng.usize(derived_objects.len().max(1)))?;
    let attr = if rng.bool() { "mass" } else { "pt" };
    Some(Requirement {
        lhs: derived_attr(&derived.name, attr),
        op: if rng.bool() { CmpOp::Gt } else { CmpOp::Lt },
        rhs: q(
            round(if attr == "mass" {
                rng.f64(5.0, 170.0)
            } else {
                rng.f64(5.0, 220.0)
            }),
            Unit::GeV,
        ),
    })
}

fn collection_predicate_requirement(object: &str, rng: &mut SplitMix64) -> Requirement {
    let attr = if rng.bool() { "pt" } else { "eta" };
    let predicate = if attr == "pt" {
        Cut {
            lhs: Expr::Attr {
                object: object.to_string(),
                attr: attr.to_string(),
            },
            op: CmpOp::Gt,
            rhs: q(round(rng.f64(5.0, 100.0)), Unit::GeV),
        }
    } else {
        Cut {
            lhs: Expr::Abs(Box::new(Expr::Attr {
                object: object.to_string(),
                attr: attr.to_string(),
            })),
            op: CmpOp::Lt,
            rhs: q(round(rng.f64(1.0, 4.7)), Unit::Dimensionless),
        }
    };
    if rng.bool() {
        Requirement {
            lhs: Expr::Any {
                object: object.to_string(),
                predicate: Box::new(predicate),
            },
            op: CmpOp::Eq,
            rhs: q(1.0, Unit::Dimensionless),
        }
    } else {
        Requirement {
            lhs: Expr::CountWhere {
                object: object.to_string(),
                predicate: Box::new(predicate),
            },
            op: CmpOp::Ge,
            rhs: q(1.0, Unit::Dimensionless),
        }
    }
}

fn histogram_expr(derived_objects: &[DerivedObjectDef], rng: &mut SplitMix64) -> Expr {
    if !derived_objects.is_empty() && rng.usize(3) != 0 {
        let derived = &derived_objects[rng.usize(derived_objects.len())];
        derived_attr(&derived.name, if rng.bool() { "mass" } else { "pt" })
    } else {
        leading_pt(OBJECTS[rng.usize(OBJECTS.len())].name)
    }
}

fn object_cuts(object: &ObjectTemplate, rng: &mut SplitMix64) -> Vec<Cut> {
    let mut attrs = object.cut_attrs.to_vec();
    for index in 0..attrs.len() {
        let swap = index + rng.usize(attrs.len() - index);
        attrs.swap(index, swap);
    }
    let cut_count = 1 + rng.usize(3);
    attrs
        .into_iter()
        .take(cut_count)
        .map(|attr| attr.cut(object.name, rng))
        .collect()
}

impl CutAttr {
    fn cut(self, object: &str, rng: &mut SplitMix64) -> Cut {
        let attr = Expr::Attr {
            object: object.to_string(),
            attr: self.name.to_string(),
        };
        let (lhs, op, value) = match self.form {
            CutForm::Greater { low, high } => (attr, CmpOp::Gt, rng.f64(low, high)),
            CutForm::Less { low, high } => (attr, CmpOp::Lt, rng.f64(low, high)),
            CutForm::AbsLess { low, high } => {
                (Expr::Abs(Box::new(attr)), CmpOp::Lt, rng.f64(low, high))
            }
        };
        Cut {
            lhs,
            op,
            rhs: Quantity {
                value: round(value),
                unit: self.unit,
            },
        }
    }
}

fn outputs_for_case(derived_case: usize) -> Vec<OutputDef> {
    let mut outputs = vec![
        output("n_muon", Expr::Count("good_muon".to_string())),
        output("n_electron", Expr::Count("good_electron".to_string())),
        output("n_jet", Expr::Count("good_jet".to_string())),
        output("lead_muon_pt", leading_pt("good_muon")),
        output("lead_electron_pt", leading_pt("good_electron")),
        output("lead_jet_pt", leading_pt("good_jet")),
        output("sum_muon_pt", sum_pt("good_muon")),
        output("sum_electron_pt", sum_pt("good_electron")),
        output("sum_jet_pt", sum_pt("good_jet")),
        output(
            "n_tight_muon",
            Expr::CountWhere {
                object: "good_muon".to_string(),
                predicate: Box::new(Cut {
                    lhs: Expr::Attr {
                        object: "good_muon".to_string(),
                        attr: "pt".to_string(),
                    },
                    op: CmpOp::Gt,
                    rhs: q(20.0, Unit::GeV),
                }),
            },
        ),
        output(
            "any_central_electron",
            Expr::Any {
                object: "good_electron".to_string(),
                predicate: Box::new(Cut {
                    lhs: Expr::Abs(Box::new(Expr::Attr {
                        object: "good_electron".to_string(),
                        attr: "eta".to_string(),
                    })),
                    op: CmpOp::Lt,
                    rhs: q(1.5, Unit::Dimensionless),
                }),
            },
        ),
        output(
            "either_lepton_pair_pt",
            Expr::EitherPairPt {
                left: "good_muon".to_string(),
                right: "good_electron".to_string(),
                leading: q(20.0, Unit::GeV),
                subleading: q(10.0, Unit::GeV),
            },
        ),
    ];
    match derived_case {
        1 => {
            outputs.push(output("dilepton_mass", derived_attr("dilepton", "mass")));
            outputs.push(output("dilepton_pt", derived_attr("dilepton", "pt")));
            outputs.push(output(
                "dilepton_min_delta_r",
                derived_attr("dilepton", "min_delta_r"),
            ));
        }
        2 => {
            outputs.push(output("dijet_mass", derived_attr("dijet", "mass")));
            outputs.push(output("dijet_pt", derived_attr("dijet", "pt")));
        }
        3 => {
            outputs.push(output("z1_mass", derived_attr("z1", "mass")));
            outputs.push(output("z2_mass", derived_attr("z2", "mass")));
            outputs.push(output("h4_mass", derived_attr("h4", "mass")));
            outputs.push(output(
                "closest_z_mass",
                Expr::ClosestMass {
                    left: "z1".to_string(),
                    right: "z2".to_string(),
                    target: q(91.2, Unit::GeV),
                },
            ));
            outputs.push(output(
                "other_z_mass",
                Expr::OtherMass {
                    left: "z1".to_string(),
                    right: "z2".to_string(),
                    target: q(91.2, Unit::GeV),
                },
            ));
        }
        4 => {
            outputs.push(output("emu_mass", derived_attr("emu", "mass")));
            outputs.push(output("emu_pt", derived_attr("emu", "pt")));
            outputs.push(output(
                "emu_min_delta_r",
                derived_attr("emu", "min_delta_r"),
            ));
        }
        _ => {}
    }
    outputs
}

fn output(name: &str, expr: Expr) -> OutputDef {
    OutputDef {
        name: name.to_string(),
        expr,
    }
}

fn leading_pt(object: &str) -> Expr {
    Expr::LeadingAttr {
        object: object.to_string(),
        attr: "pt".to_string(),
    }
}

fn sum_pt(object: &str) -> Expr {
    Expr::SumAttr {
        object: object.to_string(),
        attr: "pt".to_string(),
    }
}

fn count_requirement(object: &str, op: CmpOp, value: f64) -> Requirement {
    Requirement {
        lhs: Expr::Count(object.to_string()),
        op,
        rhs: q(value, Unit::Dimensionless),
    }
}

fn derived_attr(object: &str, attr: &str) -> Expr {
    Expr::Attr {
        object: object.to_string(),
        attr: attr.to_string(),
    }
}

fn q(value: f64, unit: Unit) -> Quantity {
    Quantity { value, unit }
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}
