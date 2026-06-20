use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_demo::GeneratedProducer;
use nano_producers::MuonProducer;

#[test]
fn generated_muon_producer_matches_handwritten_producer_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);

        let generated = GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));
        let handwritten = MuonProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        ("nMuon".to_string(), BranchColumn::U32(vec![2, 1, 2, 0, 1])),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![31.0, 10.0],
                vec![29.9],
                vec![45.0, 35.0],
                vec![],
                vec![60.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, 0.2],
                vec![0.0],
                vec![2.39, -2.0],
                vec![],
                vec![2.39],
            ]),
        ),
    ]
}
