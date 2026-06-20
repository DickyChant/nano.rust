use nano_analysis::{Ev, Features, ModelTag};
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_tagger_demo::GeneratedProducer;
use nano_inference::{MockPredictor, Tensor, TensorData};

struct MuonTagger;

impl ModelTag for MuonTagger {
    const NAME: &'static str = "muon_tagger";
    const BATCH: &'static str = "Muon";
    const OUTPUT: &'static str = "topscore";
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RefRow {
    n_good_muon: u32,
    lead_muon_topscore: f32,
}

#[test]
fn generated_muon_tagger_producer_matches_handwritten_reference_on_synthetic_events() {
    let predictor = MockPredictor::new(MuonTagger::NAME);

    for entry in 0..6 {
        let event = synthetic_event(entry);

        let generated = GeneratedProducer::analyze(&event, &predictor)
            .unwrap()
            .map(|row| RefRow {
                n_good_muon: row.n_good_muon,
                lead_muon_topscore: row.lead_muon_topscore,
            });
        let handwritten = handwritten_reference(&event, &predictor).unwrap();

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

fn handwritten_reference(
    event: &Event,
    predictor: &MockPredictor,
) -> Result<Option<RefRow>, nano_inference::InferError> {
    let muons = event.collection("Muon").map_err(core_error)?;
    let mut values = Vec::with_capacity(muons.len() * 3);
    for muon in muons.iter() {
        values.push(muon.get::<f32>("pt").map_err(core_error)?);
        values.push(muon.get::<f32>("eta").map_err(core_error)?);
        values.push(muon.get::<f32>("phi").map_err(core_error)?);
    }
    let features = Features::<MuonTagger>::from_tensors(vec![Tensor {
        name: "features".to_string(),
        shape: vec![muons.len(), 3],
        data: TensorData::F32(values),
    }]);
    let baseline = Ev::new(event).preselect(|_| true).unwrap();
    let scored = baseline.infer::<MuonTagger>(predictor, features)?;

    let mut n_good_muon = 0_u32;
    let mut lead_muon_topscore: Option<f32> = None;
    for muon in scored
        .event()
        .collection("Muon")
        .map_err(core_error)?
        .iter()
    {
        let pt = muon.get::<f32>("pt").map_err(core_error)?;
        let eta = muon.get::<f32>("eta").map_err(core_error)?;
        let topscore = scored.score(muon).map_err(core_error)?;

        if pt > 30.0 && eta.abs() < 2.4 {
            n_good_muon += 1;
            lead_muon_topscore =
                Some(lead_muon_topscore.map_or(topscore, |lead| lead.max(topscore)));
        }
    }

    let Some(lead_muon_topscore) = lead_muon_topscore else {
        return Ok(None);
    };
    if !(n_good_muon >= 1 && lead_muon_topscore > 0.5) {
        return Ok(None);
    }

    Ok(Some(RefRow {
        n_good_muon,
        lead_muon_topscore,
    }))
}

fn core_error(error: impl std::fmt::Display) -> nano_inference::InferError {
    nano_inference::InferError::Feature(error.to_string())
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        (
            "nMuon".to_string(),
            BranchColumn::U32(vec![3, 2, 2, 1, 0, 2]),
        ),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![45.0, 20.0, 36.0],
                vec![31.0, 29.0],
                vec![60.0, 42.0],
                vec![30.0],
                vec![],
                vec![80.0, 35.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, 0.2, -2.5],
                vec![2.39, 0.0],
                vec![2.41, -1.1],
                vec![0.0],
                vec![],
                vec![0.3, -0.4],
            ]),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.01, 1.5, -2.0],
                vec![0.7, -0.2],
                vec![1.2, -2.4],
                vec![0.4],
                vec![],
                vec![2.2, -1.7],
            ]),
        ),
    ]
}
