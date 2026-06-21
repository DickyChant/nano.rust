use nano_spec::{
    AnalysisSpec, CmpOp, Cut, Expr, HistogramDef, ObjectDef, OutputDef, Quantity, RegionDef,
    Requirement, SystematicDef, Unit, WeightDef, WeightSystematicDef, Year,
};

pub const FUZZ_SEED: u64 = 0x4e41_4e4f_5f44_4946;
pub const FUZZ_SPEC_COUNT: usize = 200;

#[derive(Debug, Clone)]
pub struct GeneratedSpec {
    pub index: usize,
    pub spec: AnalysisSpec,
    pub has_histogram: bool,
    pub has_weight_systematic: bool,
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

    let mut regions = vec![RegionDef {
        name: "baseline".to_string(),
        require: first_region_requirements,
    }];
    if rng.bool() {
        regions.push(RegionDef {
            name: "signal".to_string(),
            require: vec![count_requirement(
                OBJECTS[rng.usize(OBJECTS.len())].name,
                CmpOp::Ge,
                rng.f64(1.0, 3.0).floor(),
            )],
        });
    }

    let has_histogram = rng.usize(3) != 0;
    let has_weight_systematic = has_histogram && rng.bool();
    let histograms = if has_histogram {
        vec![HistogramDef {
            name: "lead_muon_pt_hist".to_string(),
            expr: leading_pt("good_muon"),
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

    GeneratedSpec {
        index,
        spec: AnalysisSpec {
            name: format!("fuzz_diff_{index:03}"),
            year: Year::Run2018,
            objects,
            derived_objects: Vec::new(),
            models: Vec::new(),
            regions,
            outputs: fixed_outputs(),
            histograms,
            weight,
            systematics,
            shape_corrections: Vec::new(),
            channels: Vec::new(),
        },
        has_histogram,
        has_weight_systematic,
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

fn fixed_outputs() -> Vec<OutputDef> {
    vec![
        output("n_muon", Expr::Count("good_muon".to_string())),
        output("n_electron", Expr::Count("good_electron".to_string())),
        output("n_jet", Expr::Count("good_jet".to_string())),
        output("lead_muon_pt", leading_pt("good_muon")),
        output("lead_electron_pt", leading_pt("good_electron")),
        output("lead_jet_pt", leading_pt("good_jet")),
        output("sum_muon_pt", sum_pt("good_muon")),
        output("sum_electron_pt", sum_pt("good_electron")),
        output("sum_jet_pt", sum_pt("good_jet")),
    ]
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
        rhs: Quantity {
            value,
            unit: Unit::Dimensionless,
        },
    }
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}
