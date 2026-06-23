use nano_spec::{
    AnalysisSpec, ChannelDef, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr, HistogramDef,
    LumiMaskDef, ModelDef, ModelOutputDType, ModelProviderKind, ModelProviderSpec,
    ObjectCandidateDef, ObjectDef, ObjectPairDef, OutputDef, PairConstraint, PairSelection,
    Quantity, RegionDef, Requirement, ScaleFactorCorrectionDef, ScaleFactorInputDef,
    ScaleFactorInputSource, ScaleFactorSystematicDef, ShapeCorrectionDef, SystematicDef, Unit,
    WeightDef, WeightSystematicDef, Year,
};

pub const FUZZ_SEED: u64 = 0x4e41_4e4f_5f44_4946;
pub const FUZZ_SPEC_COUNT: usize = 400;
pub const FUZZ_UNION_SPEC_COUNT: usize = 24;
pub const FUZZ_MODEL_HISTOGRAM_SPEC_COUNT: usize = 24;
pub const FUZZ_MODEL_WEIGHT_SYSTEMATIC_SPEC_COUNT: usize = 24;
pub const FUZZ_MODEL_SHAPE_SPEC_COUNT: usize = 24;
pub const FUZZ_DERIVED_MODEL_SPEC_COUNT: usize = 24;
pub const FUZZ_WEIGHT_SHAPE_SPEC_COUNT: usize = 24;
pub const FUZZ_SCALE_FACTOR_SPEC_COUNT: usize = 24;
pub const FUZZ_JES_SPEC_COUNT: usize = 24;
pub const FUZZ_LUMI_MASK_SPEC_COUNT: usize = 24;
pub const FUZZ_COMBINED_REAL_SPEC_COUNT: usize = 12;

const MUON_SF_PAYLOAD: &str = "../nano-spec/tests/data/muon_sf.json";
const JES_PAYLOAD: &str = "../nano-spec/tests/data/jes_uncertainty.json";
const SYNTHETIC_GOLDEN_JSON: &str = "../nano-spec/tests/data/synthetic_golden.json";

#[derive(Debug, Clone)]
pub struct GeneratedSpec {
    pub index: usize,
    pub spec: AnalysisSpec,
    pub has_histogram: bool,
    pub has_weight_systematic: bool,
    pub has_shape_correction: bool,
    pub has_derived_object: bool,
    pub has_candidate_object: bool,
    pub has_model: bool,
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

pub fn generated_union_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_UNION_SPEC_COUNT)
        .map(generated_union_spec)
        .collect()
}

pub fn generated_model_histogram_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_MODEL_HISTOGRAM_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0x9d13_781f_e2b4_6c91
                    ^ (index as u64).wrapping_mul(0xa24b_aed4_963e_e407),
            );
            let mut generated = generated_model_spec(index, &mut rng);
            let tagged = generated.spec.objects[1].name.clone();
            let score = generated.spec.models[0]
                .output
                .split_once('_')
                .expect("mock model output is source_attr")
                .1
                .to_string();
            generated.spec.histograms = vec![HistogramDef {
                name: "fuzz_model_score_hist".to_string(),
                expr: if index.is_multiple_of(2) {
                    Expr::LeadingAttr {
                        object: tagged,
                        attr: score,
                    }
                } else {
                    Expr::LeadingAttr {
                        object: generated.spec.objects[1].name.clone(),
                        attr: "pt".to_string(),
                    }
                },
                bins: 10,
                range: [0.0, if index.is_multiple_of(2) { 1.0 } else { 220.0 }],
            }];
            generated.has_histogram = true;
            generated
        })
        .collect()
}

pub fn generated_model_weight_systematic_specs() -> Vec<GeneratedSpec> {
    generated_model_histogram_specs()
        .into_iter()
        .take(FUZZ_MODEL_WEIGHT_SYSTEMATIC_SPEC_COUNT)
        .map(|mut generated| {
            let index = generated.index;
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0x51a7_5e7c_7a6b_21d3
                    ^ (index as u64).wrapping_mul(0xd6e8_feb8_6659_fd93),
            );
            generated.spec.name = format!("fuzz_model_weight_diff_{index:03}");
            generated.spec.systematics = vec![
                SystematicDef::Nominal,
                SystematicDef::Weight(WeightSystematicDef {
                    name: format!("fuzz_model_weight_{index:03}"),
                    up: round(rng.f64(1.1, 2.5)),
                    down: round(rng.f64(0.25, 0.9)),
                }),
            ];
            generated.has_weight_systematic = true;
            generated
        })
        .collect()
}

pub fn generated_model_shape_specs() -> Vec<GeneratedSpec> {
    generated_model_histogram_specs()
        .into_iter()
        .take(FUZZ_MODEL_SHAPE_SPEC_COUNT)
        .map(|mut generated| {
            let index = generated.index;
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0xa15c_0de1_5afe_47aa
                    ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
            );
            generated.spec.name = format!("fuzz_model_shape_diff_{index:03}");
            let tagged = generated.spec.objects[1].name.clone();
            generated.spec.shape_corrections = vec![ShapeCorrectionDef::fixed_scale(
                format!("fuzz_model_shape_{index:03}"),
                tagged,
                "pt".to_string(),
                round(rng.f64(1.05, 1.35)),
                round(rng.f64(0.65, 0.95)),
            )];
            generated.has_shape_correction = true;
            generated
        })
        .collect()
}

pub fn generated_derived_under_model_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_DERIVED_MODEL_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0x56e1_2d8f_c614_7a03
                    ^ (index as u64).wrapping_mul(0x94d0_49bb_1331_11eb),
            );
            let mut generated = generated_model_spec(index, &mut rng);
            generated.spec.name = format!("fuzz_derived_model_diff_{index:03}");
            let tagged = generated.spec.objects[1].name.clone();
            let score = generated.spec.models[0]
                .output
                .split_once('_')
                .expect("mock model output is source_attr")
                .1
                .to_string();

            let derived_name = if index.is_multiple_of(2) {
                "tagged_pair".to_string()
            } else {
                "tagged_candidate".to_string()
            };
            generated.spec.derived_objects = if index.is_multiple_of(2) {
                vec![DerivedObjectDef {
                    name: derived_name.clone(),
                    source: DerivedSource::Pair(ObjectPairDef {
                        object: tagged.clone(),
                        constraints: Vec::new(),
                        filters: maybe_pair_filters(&mut rng),
                        selection: PairSelection::LeadingPt,
                        exclude: Vec::new(),
                    }),
                }]
            } else {
                vec![DerivedObjectDef {
                    name: derived_name.clone(),
                    source: DerivedSource::Candidate(ObjectCandidateDef {
                        items: vec![tagged.clone(), tagged.clone()],
                        filters: maybe_candidate_filters(&mut rng),
                    }),
                }]
            };
            generated.spec.outputs.push(output(
                &format!("{derived_name}_mass"),
                derived_attr(&derived_name, "mass"),
            ));
            generated.spec.outputs.push(output(
                &format!("{derived_name}_min_delta_r"),
                derived_attr(&derived_name, "min_delta_r"),
            ));
            generated.spec.histograms = vec![HistogramDef {
                name: "fuzz_derived_model_hist".to_string(),
                expr: if index.is_multiple_of(3) {
                    Expr::LeadingAttr {
                        object: tagged,
                        attr: score,
                    }
                } else {
                    derived_attr(&derived_name, "mass")
                },
                bins: 10,
                range: [0.0, if index.is_multiple_of(3) { 1.0 } else { 220.0 }],
            }];
            generated.has_histogram = true;
            generated.has_derived_object = true;
            generated.has_candidate_object = !index.is_multiple_of(2);
            generated
        })
        .collect()
}

pub fn generated_weight_shape_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_WEIGHT_SHAPE_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0xf00d_574a_5a11_c0de
                    ^ (index as u64).wrapping_mul(0x632b_e59b_d9b4_e019),
            );
            let mut generated = generated_standard_spec(index, &mut rng);
            if generated.spec.histograms.is_empty() {
                generated.spec.histograms = vec![HistogramDef {
                    name: "fuzz_hist".to_string(),
                    expr: leading_pt(OBJECTS[index % OBJECTS.len()].name),
                    bins: 8,
                    range: [0.0, 220.0],
                }];
            }
            generated.spec.systematics = vec![
                SystematicDef::Nominal,
                SystematicDef::Weight(WeightSystematicDef {
                    name: format!("fuzz_weight_shape_{index:03}"),
                    up: round(rng.f64(1.1, 2.5)),
                    down: round(rng.f64(0.25, 0.9)),
                }),
            ];
            let shape_object = generated.spec.objects[index % generated.spec.objects.len()]
                .name
                .clone();
            generated.spec.shape_corrections = vec![ShapeCorrectionDef::fixed_scale(
                format!("fuzz_shape_weight_{index:03}"),
                shape_object,
                "pt".to_string(),
                round(rng.f64(1.02, 1.18)),
                round(rng.f64(0.82, 0.98)),
            )];
            generated.has_histogram = true;
            generated.has_weight_systematic = true;
            generated.has_shape_correction = true;
            generated
        })
        .collect()
}

pub fn generated_scale_factor_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_SCALE_FACTOR_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0x5ca1_e5fa_c70f_2026
                    ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
            );
            let template = OBJECTS[index % OBJECTS.len()];
            let collection = format!("sf_{}", template.source.to_ascii_lowercase());
            let pt_threshold = round(rng.f64(
                if template.source == "Jet" { 20.0 } else { 5.0 },
                if template.source == "Jet" {
                    120.0
                } else {
                    55.0
                },
            ));
            let eta_threshold =
                round(rng.f64(1.4, if template.source == "Jet" { 4.7 } else { 2.5 }));
            let mut cuts = vec![
                Cut {
                    lhs: Expr::Attr {
                        object: collection.clone(),
                        attr: "pt".to_string(),
                    },
                    op: CmpOp::Gt,
                    rhs: q(pt_threshold, Unit::GeV),
                },
                Cut {
                    lhs: Expr::Abs(Box::new(Expr::Attr {
                        object: collection.clone(),
                        attr: "eta".to_string(),
                    })),
                    op: CmpOp::Lt,
                    rhs: q(eta_threshold, Unit::Dimensionless),
                },
            ];
            if index.is_multiple_of(3) {
                cuts.push(Cut {
                    lhs: Expr::Attr {
                        object: collection.clone(),
                        attr: "phi".to_string(),
                    },
                    op: CmpOp::Lt,
                    rhs: q(round(rng.f64(0.5, 3.2)), Unit::Dimensionless),
                });
            }

            let mut regions = vec![RegionDef {
                name: "sf_baseline".to_string(),
                require: vec![count_requirement(&collection, CmpOp::Ge, 1.0)],
            }];
            regions.push(RegionDef {
                name: "sf_tight".to_string(),
                require: match index % 3 {
                    0 => vec![Requirement {
                        lhs: leading_pt(&collection),
                        op: CmpOp::Gt,
                        rhs: q(round(rng.f64(20.0, 90.0)), Unit::GeV),
                    }],
                    1 => vec![collection_predicate_requirement(&collection, &mut rng)],
                    _ => vec![count_requirement(
                        &collection,
                        CmpOp::Le,
                        rng.f64(2.0, 5.0).floor(),
                    )],
                },
            });

            GeneratedSpec {
                index,
                spec: AnalysisSpec {
                    name: format!("fuzz_scale_factor_diff_{index:03}"),
                    year: Year::Run2018,
                    lumi_mask: None,
                    objects: vec![ObjectDef {
                        name: collection.clone(),
                        source: template.source.to_string(),
                        cuts,
                    }],
                    derived_objects: Vec::new(),
                    models: Vec::new(),
                    regions,
                    outputs: vec![
                        output("n_sf_object", Expr::Count(collection.clone())),
                        output("lead_sf_pt", leading_pt(&collection)),
                        output("sum_sf_pt", sum_pt(&collection)),
                    ],
                    histograms: vec![HistogramDef {
                        name: "sf_weighted_lead_pt".to_string(),
                        expr: if index.is_multiple_of(2) {
                            leading_pt(&collection)
                        } else {
                            Expr::Count(collection.clone())
                        },
                        bins: 8,
                        range: [0.0, if index.is_multiple_of(2) { 220.0 } else { 8.0 }],
                    }],
                    weight: WeightDef {
                        nominal: if index.is_multiple_of(4) {
                            vec![round(rng.f64(0.7, 1.4))]
                        } else {
                            Vec::new()
                        },
                    },
                    systematics: vec![SystematicDef::Nominal],
                    shape_corrections: Vec::new(),
                    scale_factor_corrections: vec![scale_factor_correction(
                        &format!("sf_weight_{index:03}"),
                        &collection,
                    )],
                    channels: Vec::new(),
                },
                has_histogram: true,
                has_weight_systematic: true,
                has_shape_correction: false,
                has_derived_object: false,
                has_candidate_object: false,
                has_model: false,
            }
        })
        .collect()
}

pub fn generated_jes_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_JES_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0x05e5_5eed_2026_0001
                    ^ (index as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9),
            );
            let threshold = match index % 4 {
                0 => 30.0,
                1 => 40.0,
                2 => round(rng.f64(45.0, 95.0)),
                _ => round(rng.f64(95.0, 145.0)),
            };
            let collection = "jes_jet".to_string();
            let mut regions = vec![RegionDef {
                name: "jes_baseline".to_string(),
                require: vec![count_requirement(&collection, CmpOp::Ge, 1.0)],
            }];
            if index.is_multiple_of(2) {
                regions.push(RegionDef {
                    name: "jes_migration".to_string(),
                    require: vec![Requirement {
                        lhs: leading_pt(&collection),
                        op: CmpOp::Gt,
                        rhs: q(round(threshold + rng.f64(0.0, 25.0)), Unit::GeV),
                    }],
                });
            }

            GeneratedSpec {
                index,
                spec: AnalysisSpec {
                    name: format!("fuzz_jes_diff_{index:03}"),
                    year: Year::Run2018,
                    lumi_mask: None,
                    objects: vec![ObjectDef {
                        name: collection.clone(),
                        source: "Jet".to_string(),
                        cuts: vec![
                            Cut {
                                lhs: Expr::Attr {
                                    object: collection.clone(),
                                    attr: "pt".to_string(),
                                },
                                op: CmpOp::Gt,
                                rhs: q(threshold, Unit::GeV),
                            },
                            Cut {
                                lhs: Expr::Abs(Box::new(Expr::Attr {
                                    object: collection.clone(),
                                    attr: "eta".to_string(),
                                })),
                                op: CmpOp::Lt,
                                rhs: q(round(rng.f64(2.4, 5.0)), Unit::Dimensionless),
                            },
                        ],
                    }],
                    derived_objects: Vec::new(),
                    models: Vec::new(),
                    regions,
                    outputs: vec![
                        output("n_jes_jet", Expr::Count(collection.clone())),
                        output("lead_jes_pt", leading_pt(&collection)),
                        output("ht_jes", sum_pt(&collection)),
                    ],
                    histograms: vec![HistogramDef {
                        name: "jes_lead_pt".to_string(),
                        expr: if index.is_multiple_of(3) {
                            Expr::Count(collection.clone())
                        } else {
                            leading_pt(&collection)
                        },
                        bins: 10,
                        range: [0.0, if index.is_multiple_of(3) { 8.0 } else { 260.0 }],
                    }],
                    weight: WeightDef::default(),
                    systematics: vec![SystematicDef::Nominal],
                    shape_corrections: vec![jes_correction(
                        &format!("jes_total_{index:03}"),
                        &collection,
                    )],
                    scale_factor_corrections: Vec::new(),
                    channels: Vec::new(),
                },
                has_histogram: true,
                has_weight_systematic: false,
                has_shape_correction: true,
                has_derived_object: false,
                has_candidate_object: false,
                has_model: false,
            }
        })
        .collect()
}

pub fn generated_lumi_mask_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_LUMI_MASK_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0x106d_111a_5eed_2026
                    ^ (index as u64).wrapping_mul(0x94d0_49bb_1331_11eb),
            );
            let mut require = vec![
                Requirement {
                    lhs: Expr::EventScalar("HLT_IsoMu24".to_string()),
                    op: CmpOp::Eq,
                    rhs: q(1.0, Unit::Dimensionless),
                },
                Requirement {
                    lhs: Expr::EventScalar("Flag_goodVertices".to_string()),
                    op: CmpOp::Eq,
                    rhs: q(1.0, Unit::Dimensionless),
                },
            ];
            if index.is_multiple_of(2) {
                require.push(count_requirement("lumi_muon", CmpOp::Ge, 1.0));
            }
            if index.is_multiple_of(3) {
                require.push(Requirement {
                    lhs: leading_pt("lumi_muon"),
                    op: CmpOp::Gt,
                    rhs: q(round(rng.f64(4.0, 65.0)), Unit::GeV),
                });
            }
            GeneratedSpec {
                index,
                spec: AnalysisSpec {
                    name: format!("fuzz_lumi_mask_diff_{index:03}"),
                    year: Year::Run2018,
                    lumi_mask: Some(LumiMaskDef {
                        file: SYNTHETIC_GOLDEN_JSON.to_string(),
                        ranges: None,
                    }),
                    objects: vec![ObjectDef {
                        name: "lumi_muon".to_string(),
                        source: "Muon".to_string(),
                        cuts: vec![Cut {
                            lhs: Expr::Attr {
                                object: "lumi_muon".to_string(),
                                attr: "pt".to_string(),
                            },
                            op: CmpOp::Gt,
                            rhs: q(round(rng.f64(0.0, 35.0)), Unit::GeV),
                        }],
                    }],
                    derived_objects: Vec::new(),
                    models: Vec::new(),
                    regions: vec![RegionDef {
                        name: "lumi_signal".to_string(),
                        require,
                    }],
                    outputs: vec![
                        output("n_lumi_muon", Expr::Count("lumi_muon".to_string())),
                        output("lead_lumi_muon_pt", leading_pt("lumi_muon")),
                    ],
                    histograms: Vec::new(),
                    weight: WeightDef::default(),
                    systematics: vec![SystematicDef::Nominal],
                    shape_corrections: Vec::new(),
                    scale_factor_corrections: Vec::new(),
                    channels: Vec::new(),
                },
                has_histogram: false,
                has_weight_systematic: false,
                has_shape_correction: false,
                has_derived_object: false,
                has_candidate_object: false,
                has_model: false,
            }
        })
        .collect()
}

pub fn generated_combined_real_specs() -> Vec<GeneratedSpec> {
    (0..FUZZ_COMBINED_REAL_SPEC_COUNT)
        .map(|index| {
            let mut rng = SplitMix64::new(
                FUZZ_SEED
                    ^ 0xc0ed_51f1_0e55_2026
                    ^ (index as u64).wrapping_mul(0xd6e8_feb8_6659_fd93),
            );
            let muon_threshold = round(rng.f64(5.0, 45.0));
            let jet_threshold = if index.is_multiple_of(2) {
                30.0
            } else {
                round(rng.f64(35.0, 110.0))
            };
            GeneratedSpec {
                index,
                spec: AnalysisSpec {
                    name: format!("fuzz_combined_real_diff_{index:03}"),
                    year: Year::Run2018,
                    lumi_mask: Some(LumiMaskDef {
                        file: SYNTHETIC_GOLDEN_JSON.to_string(),
                        ranges: None,
                    }),
                    objects: vec![
                        ObjectDef {
                            name: "combo_muon".to_string(),
                            source: "Muon".to_string(),
                            cuts: vec![
                                Cut {
                                    lhs: Expr::Attr {
                                        object: "combo_muon".to_string(),
                                        attr: "pt".to_string(),
                                    },
                                    op: CmpOp::Gt,
                                    rhs: q(muon_threshold, Unit::GeV),
                                },
                                Cut {
                                    lhs: Expr::Abs(Box::new(Expr::Attr {
                                        object: "combo_muon".to_string(),
                                        attr: "eta".to_string(),
                                    })),
                                    op: CmpOp::Lt,
                                    rhs: q(2.4, Unit::Dimensionless),
                                },
                            ],
                        },
                        ObjectDef {
                            name: "combo_jet".to_string(),
                            source: "Jet".to_string(),
                            cuts: vec![
                                Cut {
                                    lhs: Expr::Attr {
                                        object: "combo_jet".to_string(),
                                        attr: "pt".to_string(),
                                    },
                                    op: CmpOp::Gt,
                                    rhs: q(jet_threshold, Unit::GeV),
                                },
                                Cut {
                                    lhs: Expr::Abs(Box::new(Expr::Attr {
                                        object: "combo_jet".to_string(),
                                        attr: "eta".to_string(),
                                    })),
                                    op: CmpOp::Lt,
                                    rhs: q(5.0, Unit::Dimensionless),
                                },
                            ],
                        },
                    ],
                    derived_objects: Vec::new(),
                    models: Vec::new(),
                    regions: vec![
                        RegionDef {
                            name: "combo_flags".to_string(),
                            require: vec![
                                Requirement {
                                    lhs: Expr::EventScalar("HLT_IsoMu24".to_string()),
                                    op: CmpOp::Eq,
                                    rhs: q(1.0, Unit::Dimensionless),
                                },
                                Requirement {
                                    lhs: Expr::EventScalar("Flag_goodVertices".to_string()),
                                    op: CmpOp::Eq,
                                    rhs: q(1.0, Unit::Dimensionless),
                                },
                            ],
                        },
                        RegionDef {
                            name: "combo_signal".to_string(),
                            require: vec![
                                count_requirement("combo_muon", CmpOp::Ge, 1.0),
                                count_requirement("combo_jet", CmpOp::Ge, 1.0),
                            ],
                        },
                    ],
                    outputs: vec![
                        output("n_combo_muon", Expr::Count("combo_muon".to_string())),
                        output("n_combo_jet", Expr::Count("combo_jet".to_string())),
                        output("lead_combo_jet_pt", leading_pt("combo_jet")),
                        output("combo_ht", sum_pt("combo_jet")),
                    ],
                    histograms: vec![HistogramDef {
                        name: "combo_lead_jet_pt".to_string(),
                        expr: leading_pt("combo_jet"),
                        bins: 12,
                        range: [0.0, 260.0],
                    }],
                    weight: WeightDef {
                        nominal: vec![round(rng.f64(0.8, 1.2))],
                    },
                    systematics: vec![SystematicDef::Nominal],
                    shape_corrections: vec![jes_correction(
                        &format!("combo_jes_{index:03}"),
                        "combo_jet",
                    )],
                    scale_factor_corrections: vec![scale_factor_correction(
                        &format!("combo_sf_{index:03}"),
                        "combo_muon",
                    )],
                    channels: Vec::new(),
                },
                has_histogram: true,
                has_weight_systematic: true,
                has_shape_correction: true,
                has_derived_object: false,
                has_candidate_object: false,
                has_model: false,
            }
        })
        .collect()
}

fn scale_factor_correction(name: &str, collection: &str) -> ScaleFactorCorrectionDef {
    ScaleFactorCorrectionDef {
        name: name.to_string(),
        file: MUON_SF_PAYLOAD.to_string(),
        correction: "synthetic_muon_sf".to_string(),
        collection: collection.to_string(),
        inputs: vec![
            ScaleFactorInputDef {
                name: "eta".to_string(),
                source: ScaleFactorInputSource::From("eta".to_string()),
            },
            ScaleFactorInputDef {
                name: "pt".to_string(),
                source: ScaleFactorInputSource::From("pt".to_string()),
            },
        ],
        systematic: Some(ScaleFactorSystematicDef {
            name: "scale_factors".to_string(),
            nominal: "nominal".to_string(),
            up: "systup".to_string(),
            down: "systdown".to_string(),
        }),
    }
}

fn jes_correction(name: &str, collection: &str) -> ShapeCorrectionDef {
    ShapeCorrectionDef::jes(
        name.to_string(),
        collection.to_string(),
        "pt".to_string(),
        JES_PAYLOAD.to_string(),
        "synthetic_jes_uncertainty".to_string(),
        vec![
            ScaleFactorInputDef {
                name: "eta".to_string(),
                source: ScaleFactorInputSource::From("eta".to_string()),
            },
            ScaleFactorInputDef {
                name: "pt".to_string(),
                source: ScaleFactorInputSource::From("pt".to_string()),
            },
        ],
    )
}

fn generated_spec(index: usize, rng: &mut SplitMix64) -> GeneratedSpec {
    let generated = generated_standard_spec(index, rng);
    if !generated.has_derived_object && index.is_multiple_of(3) {
        let mut model_rng =
            SplitMix64::new(FUZZ_SEED ^ (index as u64).wrapping_mul(0xd1b5_4a32_d192_ed03));
        return generated_model_spec(index, &mut model_rng);
    }
    generated
}

fn generated_union_spec(index: usize) -> GeneratedSpec {
    let muon_threshold = 5.0 + (index % 12) as f64;
    let electron_threshold = 7.0 + (index % 10) as f64;
    let jet_threshold = 20.0 + (index % 18) as f64;
    let mut channels = vec![
        union_channel("mu", "Muon", muon_threshold),
        union_channel("el", "Electron", electron_threshold),
    ];
    if index.is_multiple_of(3) {
        channels.push(union_channel("jet", "Jet", jet_threshold));
    }

    GeneratedSpec {
        index,
        spec: AnalysisSpec {
            name: format!("fuzz_union_diff_{index:03}"),
            year: Year::Run2018,
            lumi_mask: None,
            objects: Vec::new(),
            derived_objects: Vec::new(),
            models: Vec::new(),
            regions: Vec::new(),
            outputs: Vec::new(),
            histograms: vec![HistogramDef {
                name: "fuzz_union_hist".to_string(),
                expr: leading_pt("probe"),
                bins: 12,
                range: [0.0, 220.0],
            }],
            weight: WeightDef::default(),
            systematics: vec![SystematicDef::Nominal],
            shape_corrections: Vec::new(),
            scale_factor_corrections: Vec::new(),
            channels,
        },
        has_histogram: true,
        has_weight_systematic: false,
        has_shape_correction: false,
        has_derived_object: false,
        has_candidate_object: false,
        has_model: false,
    }
}

fn union_channel(name: &str, source: &str, pt_threshold: f64) -> ChannelDef {
    ChannelDef {
        name: name.to_string(),
        objects: vec![ObjectDef {
            name: "probe".to_string(),
            source: source.to_string(),
            cuts: vec![Cut {
                lhs: Expr::Attr {
                    object: "probe".to_string(),
                    attr: "pt".to_string(),
                },
                op: CmpOp::Gt,
                rhs: q(round(pt_threshold), Unit::GeV),
            }],
        }],
        derived_objects: Vec::new(),
        regions: vec![RegionDef {
            name: "selected".to_string(),
            require: vec![count_requirement("probe", CmpOp::Ge, 1.0)],
        }],
        outputs: vec![
            output("n_probe", Expr::Count("probe".to_string())),
            output("lead_probe_pt", leading_pt("probe")),
        ],
    }
}

fn generated_standard_spec(index: usize, rng: &mut SplitMix64) -> GeneratedSpec {
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
    if rng.usize(3) == 0 {
        first_region_requirements.push(sum_pt_requirement(
            OBJECTS[rng.usize(OBJECTS.len())].name,
            rng,
        ));
    }
    if rng.usize(3) == 0 {
        first_region_requirements.push(leading_pt_requirement(
            OBJECTS[rng.usize(OBJECTS.len())].name,
            rng,
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
        if rng.usize(3) == 0 {
            require.push(sum_pt_requirement(object, rng));
        }
        if rng.usize(3) == 0 {
            require.push(leading_pt_requirement(object, rng));
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
        vec![ShapeCorrectionDef::fixed_scale(
            format!("fuzz_shape_{index:03}"),
            OBJECTS[rng.usize(OBJECTS.len())].name.to_string(),
            "pt".to_string(),
            round(rng.f64(1.02, 1.18)),
            round(rng.f64(0.82, 0.98)),
        )]
    } else {
        Vec::new()
    };

    GeneratedSpec {
        index,
        spec: AnalysisSpec {
            name: format!("fuzz_diff_{index:03}"),
            year: Year::Run2018,
            lumi_mask: None,
            objects,
            derived_objects,
            models: Vec::new(),
            regions,
            outputs: outputs_for_case(derived_case),
            histograms,
            weight,
            systematics,
            shape_corrections,
            scale_factor_corrections: Vec::new(),
            channels: Vec::new(),
        },
        has_histogram,
        has_weight_systematic,
        has_shape_correction,
        has_derived_object,
        has_candidate_object,
        has_model: false,
    }
}

fn generated_model_spec(index: usize, rng: &mut SplitMix64) -> GeneratedSpec {
    let target = if rng.bool() {
        ModelTarget {
            source: "Muon",
            selected: "good_muon",
            tagged: "tagged_muon",
            score: "topscore",
            inputs: &["Muon_pt", "Muon_eta", "Muon_phi"],
            base_cuts: MUON_CUTS,
            pt_low: 5.0,
            pt_high: 45.0,
        }
    } else {
        ModelTarget {
            source: "Jet",
            selected: "good_jet",
            tagged: "tagged_jet",
            score: "mock_score",
            inputs: &["Jet_pt", "Jet_eta", "Jet_phi", "Jet_mass"],
            base_cuts: JET_CUTS,
            pt_low: 20.0,
            pt_high: 110.0,
        }
    };
    let score_cut = round(rng.f64(0.15, 0.75));
    let leading_score_cut = round((score_cut - rng.f64(0.05, 0.12)).max(0.0));

    let selected = ObjectDef {
        name: target.selected.to_string(),
        source: target.source.to_string(),
        cuts: object_cuts(
            &ObjectTemplate {
                name: target.selected,
                source: target.source,
                cut_attrs: target.base_cuts,
            },
            rng,
        ),
    };
    let tagged = ObjectDef {
        name: target.tagged.to_string(),
        source: target.source.to_string(),
        cuts: vec![
            Cut {
                lhs: Expr::Attr {
                    object: target.tagged.to_string(),
                    attr: "pt".to_string(),
                },
                op: CmpOp::Gt,
                rhs: q(round(rng.f64(target.pt_low, target.pt_high)), Unit::GeV),
            },
            Cut {
                lhs: Expr::Attr {
                    object: target.tagged.to_string(),
                    attr: target.score.to_string(),
                },
                op: CmpOp::Gt,
                rhs: q(score_cut, Unit::Dimensionless),
            },
        ],
    };

    GeneratedSpec {
        index,
        spec: AnalysisSpec {
            name: format!("fuzz_model_diff_{index:03}"),
            year: Year::Run2018,
            lumi_mask: None,
            objects: vec![selected, tagged],
            derived_objects: Vec::new(),
            models: vec![ModelDef {
                name: format!(
                    "fuzz_{}_tagger_{index:03}",
                    target.source.to_ascii_lowercase()
                ),
                inputs: target
                    .inputs
                    .iter()
                    .map(|input| (*input).to_string())
                    .collect(),
                output: format!("{}_{}", target.source, target.score),
                output_dtype: ModelOutputDType::F32,
                batch: target.source.to_string(),
                provider: ModelProviderSpec {
                    kind: ModelProviderKind::Mock,
                    endpoint: None,
                    launch: None,
                    onnx_path: None,
                },
            }],
            regions: vec![RegionDef {
                name: "model_signal".to_string(),
                require: vec![
                    count_requirement(target.tagged, CmpOp::Ge, 1.0),
                    Requirement {
                        lhs: Expr::LeadingAttr {
                            object: target.tagged.to_string(),
                            attr: target.score.to_string(),
                        },
                        op: CmpOp::Gt,
                        rhs: q(leading_score_cut, Unit::Dimensionless),
                    },
                ],
            }],
            outputs: vec![
                output(
                    &format!("n_{}", target.selected),
                    Expr::Count(target.selected.to_string()),
                ),
                output(
                    &format!("n_{}", target.tagged),
                    Expr::Count(target.tagged.to_string()),
                ),
                output(
                    &format!("lead_{}_pt", target.tagged),
                    Expr::LeadingAttr {
                        object: target.tagged.to_string(),
                        attr: "pt".to_string(),
                    },
                ),
                output(
                    &format!("lead_{}_{}", target.tagged, target.score),
                    Expr::LeadingAttr {
                        object: target.tagged.to_string(),
                        attr: target.score.to_string(),
                    },
                ),
            ],
            histograms: Vec::new(),
            weight: WeightDef::default(),
            systematics: vec![SystematicDef::Nominal],
            shape_corrections: Vec::new(),
            scale_factor_corrections: Vec::new(),
            channels: Vec::new(),
        },
        has_histogram: false,
        has_weight_systematic: false,
        has_shape_correction: false,
        has_derived_object: false,
        has_candidate_object: false,
        has_model: true,
    }
}

#[derive(Debug, Clone, Copy)]
struct ModelTarget {
    source: &'static str,
    selected: &'static str,
    tagged: &'static str,
    score: &'static str,
    inputs: &'static [&'static str],
    base_cuts: &'static [CutAttr],
    pt_low: f64,
    pt_high: f64,
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

fn sum_pt_requirement(object: &str, rng: &mut SplitMix64) -> Requirement {
    Requirement {
        lhs: sum_pt(object),
        op: if rng.bool() { CmpOp::Gt } else { CmpOp::Le },
        rhs: q(round(rng.f64(30.0, 360.0)), Unit::GeV),
    }
}

fn leading_pt_requirement(object: &str, rng: &mut SplitMix64) -> Requirement {
    Requirement {
        lhs: leading_pt(object),
        op: if rng.bool() { CmpOp::Gt } else { CmpOp::Le },
        rhs: q(round(rng.f64(15.0, 180.0)), Unit::GeV),
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
