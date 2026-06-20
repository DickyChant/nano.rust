use nano_analysis::{
    fill, passes_muon_signal_selection, select_muon_signal_region, Ev, EventWeight, Features,
    Hist1D, ModelTag, SignalRegion,
};
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_inference::{MockPredictor, Tensor, TensorData};

struct MuonTagger;

impl ModelTag for MuonTagger {
    const NAME: &'static str = "muon_tagger";
    const BATCH: &'static str = "Muon";
    const OUTPUT: &'static str = "topscore";
}

fn muon_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .unwrap()
}

fn muon_event(muon_pts: Vec<f32>, muon_etas: Vec<f32>) -> Event {
    Event::from_columns(
        muon_schema(),
        [
            ("Muon_pt", BranchColumn::VecF32(vec![muon_pts])),
            ("Muon_eta", BranchColumn::VecF32(vec![muon_etas])),
        ],
        0,
    )
    .unwrap()
}

#[test]
fn raw_to_signal_weighted_event_fills_histogram() {
    let event = muon_event(vec![24.0, 45.0], vec![0.1, -1.2]);
    let weighted = Ev::new(&event)
        .preselect(|event| !event.collection("Muon").unwrap().is_empty())
        .unwrap()
        .select::<SignalRegion>(|event| passes_muon_signal_selection(event).unwrap())
        .unwrap()
        .weight(EventWeight::nominal().times(2.5));

    let mut hist = Hist1D::new(4, 0.0, 100.0);
    fill(&mut hist, &weighted, 45.0);

    assert_eq!(weighted.region_name(), "signal");
    assert_eq!(weighted.weight().value(), 2.5);
    assert_eq!(hist.bins(), &[0.0, 2.5, 0.0, 0.0]);
    assert_eq!(hist.underflow(), 0.0);
    assert_eq!(hist.overflow(), 0.0);
    assert_eq!(hist.sumw(), 2.5);
}

#[test]
fn vetoed_event_yields_none_before_fill_token_exists() {
    let event = muon_event(vec![25.0, 29.0], vec![0.1, 1.5]);
    let selected = Ev::new(&event)
        .preselect(|_| true)
        .unwrap()
        .select::<SignalRegion>(|event| passes_muon_signal_selection(event).unwrap());

    assert!(selected.is_none());
}

#[test]
fn typed_muon_selection_matches_existing_cut_shape() {
    let passing = muon_event(vec![20.0, 31.0], vec![0.0, 2.39]);
    let failing_pt = muon_event(vec![20.0, 30.0], vec![0.0, 1.0]);
    let failing_eta = muon_event(vec![40.0], vec![2.4]);

    assert!(select_muon_signal_region(Ev::new(&passing)).is_some());
    assert!(select_muon_signal_region(Ev::new(&failing_pt)).is_none());
    assert!(select_muon_signal_region(Ev::new(&failing_eta)).is_none());
}

#[test]
fn inference_transition_attaches_typed_score() {
    let event = muon_event(vec![24.0, 45.0], vec![0.1, -1.2]);
    let baseline = Ev::new(&event).preselect(|_| true).unwrap();
    let features = Features::<MuonTagger>::from_tensors(vec![Tensor {
        name: "features".to_string(),
        shape: vec![2, 2],
        data: TensorData::F32(vec![24.0, 0.1, 45.0, -1.2]),
    }]);

    let scored = baseline
        .infer::<MuonTagger>(&MockPredictor::new(MuonTagger::NAME), features)
        .unwrap();
    let muons = scored.event().collection("Muon").unwrap();
    let first_score = scored.score(muons.get(0).unwrap()).unwrap();
    let second_score = scored.score(muons.get(1).unwrap()).unwrap();

    assert!((0.0..=1.0).contains(&first_score));
    assert!((0.0..=1.0).contains(&second_score));
    assert_eq!(
        first_score,
        muons.get(0).unwrap().get::<f32>("topscore").unwrap()
    );
    assert_ne!(first_score, second_score);
}
