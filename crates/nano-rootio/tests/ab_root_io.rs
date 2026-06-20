use std::path::{Path, PathBuf};

use futures::executor::block_on;
use futures::StreamExt;
use nano_rootio::write::{write_tree, Branch};
use nano_rootio::{ColumnData, ColumnRequest, RootFile};

type NomResult<'a, T> = nom::IResult<&'a [u8], T, nom::error::Error<&'a [u8]>>;

fn be_i32(input: &[u8]) -> NomResult<'_, i32> {
    nom::number::complete::be_i32(input)
}

fn be_i16(input: &[u8]) -> NomResult<'_, i16> {
    nom::number::complete::be_i16(input)
}

fn be_i8(input: &[u8]) -> NomResult<'_, i8> {
    nom::number::complete::be_i8(input)
}

fn be_i64(input: &[u8]) -> NomResult<'_, i64> {
    nom::number::complete::be_i64(input)
}

fn be_u32(input: &[u8]) -> NomResult<'_, u32> {
    nom::number::complete::be_u32(input)
}

fn be_u16(input: &[u8]) -> NomResult<'_, u16> {
    nom::number::complete::be_u16(input)
}

fn be_u8(input: &[u8]) -> NomResult<'_, u8> {
    nom::number::complete::be_u8(input)
}

fn be_u64(input: &[u8]) -> NomResult<'_, u64> {
    nom::number::complete::be_u64(input)
}

fn be_f32(input: &[u8]) -> NomResult<'_, f32> {
    nom::number::complete::be_f32(input)
}

fn be_f64(input: &[u8]) -> NomResult<'_, f64> {
    nom::number::complete::be_f64(input)
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn read_rootio<T, P>(path: &Path, tree_name: &str, branch_name: &str, parser: P) -> Vec<T>
where
    T: 'static,
    P: for<'a> Fn(&'a [u8]) -> nom::IResult<&'a [u8], T> + Copy,
{
    block_on(async {
        let file = root_io::RootFile::new(path).await.expect("root-io open");
        let tree = file
            .items()
            .iter()
            .find(|item| {
                item.name().contains(&format!("`{tree_name}`"))
                    && item.verbose_info().contains("TTree")
            })
            .expect("root-io tree")
            .as_tree()
            .await
            .expect("root-io parse tree");
        tree.branch_by_name(branch_name)
            .expect("root-io branch")
            .as_fixed_size_iterator(parser)
            .collect::<Vec<_>>()
            .await
    })
}

fn read_rootio_range<T, P>(
    path: &Path,
    tree_name: &str,
    branch_name: &str,
    parser: P,
    start: usize,
    len: usize,
) -> Vec<T>
where
    T: 'static,
    P: for<'a> Fn(&'a [u8]) -> nom::IResult<&'a [u8], T> + Copy,
{
    block_on(async {
        let file = root_io::RootFile::new(path).await.expect("root-io open");
        let tree = file
            .items()
            .iter()
            .find(|item| {
                item.name().contains(&format!("`{tree_name}`"))
                    && item.verbose_info().contains("TTree")
            })
            .expect("root-io tree")
            .as_tree()
            .await
            .expect("root-io parse tree");
        tree.branch_by_name(branch_name)
            .expect("root-io branch")
            .as_fixed_size_iterator(parser)
            .skip(start)
            .take(len)
            .collect::<Vec<_>>()
            .await
    })
}

fn read_rootio_jagged<T, P>(
    path: &Path,
    tree_name: &str,
    branch_name: &str,
    counter_name: &str,
    parser: P,
) -> Vec<Vec<T>>
where
    T: 'static,
    P: for<'a> Fn(&'a [u8]) -> nom::IResult<&'a [u8], T> + Copy,
{
    block_on(async {
        let file = root_io::RootFile::new(path).await.expect("root-io open");
        let tree = file
            .items()
            .iter()
            .find(|item| {
                item.name().contains(&format!("`{tree_name}`"))
                    && item.verbose_info().contains("TTree")
            })
            .expect("root-io tree")
            .as_tree()
            .await
            .expect("root-io parse tree");
        let counts: Vec<u32> = tree
            .branch_by_name(counter_name)
            .expect("root-io counter branch")
            .as_fixed_size_iterator(|i| be_u32(i))
            .collect()
            .await;
        tree.branch_by_name(branch_name)
            .expect("root-io jagged branch")
            .as_var_size_iterator(parser, counts)
            .collect::<Vec<_>>()
            .await
    })
}

fn read_rootio_jagged_range<T, P>(
    path: &Path,
    tree_name: &str,
    branch_name: &str,
    counter_name: &str,
    parser: P,
    start: usize,
    len: usize,
) -> Vec<Vec<T>>
where
    T: 'static,
    P: for<'a> Fn(&'a [u8]) -> nom::IResult<&'a [u8], T> + Copy,
{
    block_on(async {
        let file = root_io::RootFile::new(path).await.expect("root-io open");
        let tree = file
            .items()
            .iter()
            .find(|item| {
                item.name().contains(&format!("`{tree_name}`"))
                    && item.verbose_info().contains("TTree")
            })
            .expect("root-io tree")
            .as_tree()
            .await
            .expect("root-io parse tree");
        let counts: Vec<u32> = tree
            .branch_by_name(counter_name)
            .expect("root-io counter branch")
            .as_fixed_size_iterator(|i| be_u32(i))
            .take(start + len)
            .collect()
            .await;
        tree.branch_by_name(branch_name)
            .expect("root-io jagged branch")
            .as_var_size_iterator(parser, counts)
            .skip(start)
            .take(len)
            .collect::<Vec<_>>()
            .await
    })
}

fn assert_branch_eq<T, P>(path: &Path, tree_name: &str, branch_name: &str, parser: P)
where
    T: nano_rootio::Scalar + 'static,
    P: for<'a> Fn(&'a [u8]) -> nom::IResult<&'a [u8], T> + Copy,
{
    let file = RootFile::open(path).expect("nano-rootio open");
    let tree = file.tree(tree_name).expect("nano-rootio tree");
    let expected = read_rootio(path, tree_name, branch_name, parser);
    let actual = tree
        .read_scalar::<T>(branch_name)
        .expect("nano-rootio scalar branch");
    assert_eq!(
        actual,
        expected,
        "{branch_name} differs in {}",
        path.display()
    );
}

#[test]
fn opens_lists_objects_and_streamers() {
    let path = repo_path("crates/root-io/src/test_data/simple.root");
    let file = RootFile::open(&path).unwrap();
    let objects = file.objects();
    assert!(
        objects
            .iter()
            .any(|object| object.name() == "tree" && object.class() == "TTree"),
        "objects: {objects:?}"
    );
    assert!(
        !file.streamer_info_names().is_empty(),
        "expected parsed streamer info names"
    );
}

#[test]
fn ab_simple_root_scalars() {
    let path = repo_path("crates/root-io/src/test_data/simple.root");
    assert_branch_eq::<i32, _>(&path, "tree", "one", be_i32);
    assert_branch_eq::<f32, _>(&path, "tree", "two", be_f32);
}

#[test]
fn ab_small_flat_tree_scalars() {
    let path = repo_path("crates/root-io/src/test_data/small-flat-tree.root");
    assert_branch_eq::<i32, _>(&path, "tree", "Int32", be_i32);
    assert_branch_eq::<i64, _>(&path, "tree", "Int64", be_i64);
    assert_branch_eq::<u32, _>(&path, "tree", "UInt32", be_u32);
    assert_branch_eq::<u64, _>(&path, "tree", "UInt64", be_u64);
    assert_branch_eq::<f32, _>(&path, "tree", "Float32", be_f32);
}

#[test]
fn ab_zmumu_scalars() {
    let path = repo_path("crates/root-io/src/test_data/Zmumu.root");
    assert_branch_eq::<i32, _>(&path, "events", "Run", be_i32);
    assert_branch_eq::<i32, _>(&path, "events", "Event", be_i32);
    assert_branch_eq::<i32, _>(&path, "events", "Q1", be_i32);
    assert_branch_eq::<i32, _>(&path, "events", "Q2", be_i32);
}

#[test]
fn ab_real_nanoaod_scalars_if_present() {
    let path = repo_path("tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root");
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return;
    }
    assert_branch_eq::<u32, _>(&path, "Events", "nMuon", be_u32);
    assert_branch_eq::<f32, _>(&path, "Events", "MET_pt", be_f32);
    assert_branch_eq::<u32, _>(&path, "Events", "run", be_u32);
    assert_branch_eq::<u64, _>(&path, "Events", "event", be_u64);
}

#[test]
fn ab_written_jagged_f32_and_auto_counter() {
    let path = temp_root_path("nano-rootio-written-jagged.root");
    let _ = std::fs::remove_file(&path);
    let counts = vec![0_u32, 2, 1, 3, 0, 2];
    let pts = vec![
        Vec::new(),
        vec![10.0_f32, 11.5],
        vec![-1.25],
        vec![2.0, 4.0, 8.0],
        Vec::new(),
        vec![42.0, 43.5],
    ];
    write_tree(
        &path,
        "Events",
        &[
            Branch::u32("nJet", counts.clone()),
            Branch::vec_f32("Jet_pt", pts.clone()),
        ],
    )
    .expect("write synthetic jagged ROOT file");

    let file = RootFile::open(&path).expect("nano-rootio open");
    let tree = file.tree("Events").expect("nano-rootio tree");
    let expected = read_rootio_jagged(&path, "Events", "Jet_pt", "nJet", be_f32);
    let explicit = tree
        .read_jagged::<f32>("Jet_pt", "nJet")
        .expect("explicit jagged read");
    let auto = tree
        .read_jagged_auto::<f32>("Jet_pt")
        .expect("auto jagged read");
    assert_eq!(explicit, expected);
    assert_eq!(auto, expected);
    assert_eq!(auto, pts);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn nano_rootio_writer_roundtrips_scalars_and_jagged_object_refs() {
    let path = temp_root_path("nano-rootio-owned-writer-roundtrip.root");
    let _ = std::fs::remove_file(&path);

    let u8_values = vec![0_u8, 1, 127, 255, 42, 9];
    let i8_values = vec![-128_i8, -1, 0, 1, 42, 127];
    let u16_values = vec![0_u16, 1, 255, 1024, 65535, 17];
    let i16_values = vec![-32768_i16, -2, 0, 2, 1234, 32767];
    let u32_values = vec![0_u32, 1, 4_000_000_000, 17, 33, 99];
    let i32_values = vec![-2_000_000_000_i32, -7, 0, 42, 123456, i32::MAX];
    let u64_values = vec![0_u64, 1, 9_000_000_000, u64::MAX - 5, 17, 99];
    let i64_values = vec![i64::MIN + 5, -9_000_000_000, 0, 1, 42, i64::MAX];
    let f32_values = vec![1.25_f32, -2.5, 0.0, 3.75, f32::INFINITY, -0.0];
    let bool_values = vec![true, false, true, true, false, false];
    let counts = vec![0_u32, 2, 1, 8, 0, 33];
    let jet_pt = vec![
        Vec::new(),
        vec![10.0_f32, 11.5],
        vec![-1.25],
        (0..8).map(|index| index as f32 + 0.5).collect(),
        Vec::new(),
        (0..33).map(|index| 100.0 + index as f32).collect(),
    ];
    let jet_charge = vec![
        Vec::new(),
        vec![-1_i16, 1],
        vec![0],
        vec![1, 1, -1, 0, 2, -2, 3, -3],
        Vec::new(),
        (0..33).map(|index| index as i16 - 16).collect(),
    ];
    let jet_id = vec![
        Vec::new(),
        vec![1_u32, 2],
        vec![3],
        (10_u32..18).collect(),
        Vec::new(),
        (100_u32..133).collect(),
    ];

    write_tree(
        &path,
        "Events",
        &[
            Branch::u8("u8_branch", u8_values.clone()),
            Branch::i8("i8_branch", i8_values.clone()),
            Branch::u16("u16_branch", u16_values.clone()),
            Branch::i16("i16_branch", i16_values.clone()),
            Branch::u32("u32_branch", u32_values.clone()),
            Branch::i32("i32_branch", i32_values.clone()),
            Branch::u64("u64_branch", u64_values.clone()),
            Branch::i64("i64_branch", i64_values.clone()),
            Branch::f32("f32_branch", f32_values.clone()),
            Branch::bool("bool_branch", bool_values.clone()),
            Branch::u32("nJet", counts.clone()),
            Branch::vec_f32("Jet_pt", jet_pt.clone()),
            Branch::vec_i16("Jet_charge", jet_charge.clone()),
            Branch::vec_u32("Jet_id", jet_id.clone()),
        ],
    )
    .expect("write owned ROOT file");

    let file = RootFile::open(&path).expect("nano-rootio open");
    let tree = file.tree("Events").expect("nano-rootio tree");
    assert_eq!(tree.read_scalar::<u8>("u8_branch").unwrap(), u8_values);
    assert_eq!(tree.read_scalar::<i8>("i8_branch").unwrap(), i8_values);
    assert_eq!(tree.read_scalar::<u16>("u16_branch").unwrap(), u16_values);
    assert_eq!(tree.read_scalar::<i16>("i16_branch").unwrap(), i16_values);
    assert_eq!(tree.read_scalar::<u32>("u32_branch").unwrap(), u32_values);
    assert_eq!(tree.read_scalar::<i32>("i32_branch").unwrap(), i32_values);
    assert_eq!(tree.read_scalar::<u64>("u64_branch").unwrap(), u64_values);
    assert_eq!(tree.read_scalar::<i64>("i64_branch").unwrap(), i64_values);
    assert_eq!(tree.read_scalar::<f32>("f32_branch").unwrap(), f32_values);
    assert_eq!(
        tree.read_scalar::<bool>("bool_branch").unwrap(),
        bool_values
    );
    assert_eq!(tree.read_scalar::<u32>("nJet").unwrap(), counts);
    assert_eq!(tree.read_jagged_auto::<f32>("Jet_pt").unwrap(), jet_pt);
    assert_eq!(
        tree.read_jagged_auto::<i16>("Jet_charge").unwrap(),
        jet_charge
    );
    assert_eq!(tree.read_jagged_auto::<u32>("Jet_id").unwrap(), jet_id);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn nano_rootio_writer_scalars_are_readable_by_vendored_root_io() {
    let path = temp_root_path("nano-rootio-owned-writer-vendored-read.root");
    let _ = std::fs::remove_file(&path);

    let u8_values = vec![0_u8, 1, 255, 42];
    let i8_values = vec![-128_i8, -1, 0, 127];
    let u16_values = vec![0_u16, 1, 65535, 42];
    let i16_values = vec![-32768_i16, -1, 0, 32767];
    let u32_values = vec![0_u32, 1, 4_000_000_000, 17];
    let i32_values = vec![-7_i32, 0, 42, 123456];
    let u64_values = vec![0_u64, 1, 9_000_000_000, u64::MAX - 5];
    let i64_values = vec![i64::MIN + 5, -1, 0, i64::MAX];
    let f32_values = vec![1.25_f32, -2.5, 3.75, 8.0];
    let f64_values = vec![1.25_f64, -2.5, 3.75, 8.0];
    let bool_values = vec![true, false, true, true];

    write_tree(
        &path,
        "Events",
        &[
            Branch::u8("u8_branch", u8_values.clone()),
            Branch::i8("i8_branch", i8_values.clone()),
            Branch::u16("u16_branch", u16_values.clone()),
            Branch::i16("i16_branch", i16_values.clone()),
            Branch::u32("u32_branch", u32_values.clone()),
            Branch::i32("i32_branch", i32_values.clone()),
            Branch::u64("u64_branch", u64_values.clone()),
            Branch::i64("i64_branch", i64_values.clone()),
            Branch::f32("f32_branch", f32_values.clone()),
            Branch::f64("f64_branch", f64_values.clone()),
            Branch::bool("bool_branch", bool_values.clone()),
        ],
    )
    .expect("write owned ROOT file");

    assert_eq!(read_rootio(&path, "Events", "u8_branch", be_u8), u8_values);
    assert_eq!(read_rootio(&path, "Events", "i8_branch", be_i8), i8_values);
    assert_eq!(
        read_rootio(&path, "Events", "u16_branch", be_u16),
        u16_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "i16_branch", be_i16),
        i16_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "u32_branch", be_u32),
        u32_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "i32_branch", be_i32),
        i32_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "u64_branch", be_u64),
        u64_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "i64_branch", be_i64),
        i64_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "f32_branch", be_f32),
        f32_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "f64_branch", be_f64),
        f64_values
    );
    assert_eq!(
        read_rootio(&path, "Events", "bool_branch", |i| {
            be_u8(i).map(|(i, value)| (i, value != 0))
        }),
        bool_values
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn windowed_scalar_and_jagged_match_full_slices() {
    let path = temp_root_path("nano-rootio-windowed.root");
    let _ = std::fs::remove_file(&path);
    let counts = vec![1_u32, 0, 3, 2, 0, 1, 2, 4];
    let pts = vec![
        vec![1.0_f32],
        Vec::new(),
        vec![2.0, 3.0, 4.0],
        vec![5.0, 6.0],
        Vec::new(),
        vec![7.0],
        vec![8.0, 9.0],
        vec![10.0, 11.0, 12.0, 13.0],
    ];
    let runs = vec![100_u32, 101, 102, 103, 104, 105, 106, 107];
    write_tree(
        &path,
        "Events",
        &[
            Branch::u32("run", runs.clone()),
            Branch::u32("nJet", counts),
            Branch::vec_f32("Jet_pt", pts.clone()),
        ],
    )
    .expect("write synthetic window ROOT file");

    let file = RootFile::open(&path).expect("nano-rootio open");
    let tree = file.tree("Events").expect("nano-rootio tree");
    let scalar_full = tree.read_scalar::<u32>("run").expect("full scalar");
    let jagged_full = tree
        .read_jagged::<f32>("Jet_pt", "nJet")
        .expect("full jagged");
    let scalar_window = tree
        .read_scalar_range::<u32>("run", 2, 4)
        .expect("scalar window");
    let jagged_window = tree
        .read_jagged_range::<f32>("Jet_pt", "nJet", 2, 4)
        .expect("jagged window");

    assert_eq!(scalar_window, scalar_full[2..6]);
    assert_eq!(jagged_window, jagged_full[2..6]);
    assert_eq!(
        scalar_window,
        read_rootio_range(&path, "Events", "run", be_u32, 2, 4)
    );
    assert_eq!(
        jagged_window,
        read_rootio_jagged_range(&path, "Events", "Jet_pt", "nJet", be_f32, 2, 4)
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn streamed_chunks_are_value_identical_to_windowed_reads() {
    let path = temp_root_path("nano-rootio-streamed.root");
    let _ = std::fs::remove_file(&path);
    let counts = vec![0_u32, 1, 2, 0, 3, 1, 0];
    let pts = vec![
        Vec::new(),
        vec![1.0_f32],
        vec![2.0, 3.0],
        Vec::new(),
        vec![4.0, 5.0, 6.0],
        vec![7.0],
        Vec::new(),
    ];
    let met = vec![20.0_f32, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0];
    write_tree(
        &path,
        "Events",
        &[
            Branch::f32("MET_pt", met.clone()),
            Branch::u32("nMuon", counts),
            Branch::vec_f32("Muon_pt", pts.clone()),
        ],
    )
    .expect("write synthetic stream ROOT file");

    let file = RootFile::open(&path).expect("nano-rootio open");
    let tree = file.tree("Events").expect("nano-rootio tree");
    let requests = vec![
        ColumnRequest::ScalarF32("MET_pt".to_string()),
        ColumnRequest::JaggedF32 {
            branch: "Muon_pt".to_string(),
            counter: "nMuon".to_string(),
        },
    ];
    let mut streamed_met = Vec::new();
    let mut streamed_muons = Vec::new();
    for chunk in tree
        .chunked_reader(1, 5, 2, requests)
        .expect("chunked reader")
    {
        let chunk = chunk.expect("stream chunk");
        assert!(chunk.len <= 2);
        match &chunk.columns[0].data {
            ColumnData::F32(values) => streamed_met.extend_from_slice(values),
            other => panic!("unexpected MET column {other:?}"),
        }
        match &chunk.columns[1].data {
            ColumnData::JaggedF32(values) => streamed_muons.extend(values.clone()),
            other => panic!("unexpected Muon_pt column {other:?}"),
        }
    }

    assert_eq!(
        streamed_met,
        tree.read_scalar_range::<f32>("MET_pt", 1, 5).unwrap()
    );
    assert_eq!(
        streamed_muons,
        tree.read_jagged_range::<f32>("Muon_pt", "nMuon", 1, 5)
            .unwrap()
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn ab_real_nanoaod_jagged_and_windowed_streaming_if_present() {
    let path = repo_path("tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root");
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return;
    }
    let file = RootFile::open(&path).expect("nano-rootio open");
    let tree = file.tree("Events").expect("nano-rootio tree");

    let expected_n_muon = read_rootio_range(&path, "Events", "nMuon", be_u32, 0, 32);
    let expected_muon_pt =
        read_rootio_jagged_range(&path, "Events", "Muon_pt", "nMuon", be_f32, 0, 32);
    let expected_muon_eta =
        read_rootio_jagged_range(&path, "Events", "Muon_eta", "nMuon", be_f32, 0, 32);
    assert_eq!(
        tree.read_scalar_range::<u32>("nMuon", 0, 32).unwrap(),
        expected_n_muon
    );
    assert_eq!(
        tree.read_jagged_range::<f32>("Muon_pt", "nMuon", 0, 32)
            .unwrap(),
        expected_muon_pt
    );
    assert_eq!(
        tree.read_jagged_range_auto::<f32>("Muon_eta", 0, 32)
            .unwrap(),
        expected_muon_eta
    );

    let scalar_full_prefix = tree.read_scalar_range::<u32>("run", 0, 256).unwrap();
    let scalar_inner = tree.read_scalar_range::<u32>("run", 111, 23).unwrap();
    assert_eq!(scalar_inner, scalar_full_prefix[111..134]);
    assert_eq!(
        scalar_inner,
        read_rootio_range(&path, "Events", "run", be_u32, 111, 23)
    );

    let jagged_full_prefix = tree
        .read_jagged_range::<f32>("Muon_pt", "nMuon", 0, 256)
        .unwrap();
    let jagged_inner = tree
        .read_jagged_range::<f32>("Muon_pt", "nMuon", 111, 23)
        .unwrap();
    assert_eq!(jagged_inner, jagged_full_prefix[111..134]);
    assert_eq!(
        jagged_inner,
        read_rootio_jagged_range(&path, "Events", "Muon_pt", "nMuon", be_f32, 111, 23)
    );

    let requests = vec![
        ColumnRequest::ScalarU32("nMuon".to_string()),
        ColumnRequest::JaggedF32 {
            branch: "Muon_pt".to_string(),
            counter: "nMuon".to_string(),
        },
    ];
    let mut streamed_n_muon = Vec::new();
    let mut streamed_muon_pt = Vec::new();
    for chunk in tree
        .chunked_reader(0, 32, 7, requests)
        .expect("real NanoAOD chunked reader")
    {
        let chunk = chunk.expect("real NanoAOD chunk");
        match &chunk.columns[0].data {
            ColumnData::U32(values) => streamed_n_muon.extend_from_slice(values),
            other => panic!("unexpected nMuon column {other:?}"),
        }
        match &chunk.columns[1].data {
            ColumnData::JaggedF32(values) => streamed_muon_pt.extend(values.clone()),
            other => panic!("unexpected Muon_pt column {other:?}"),
        }
    }
    assert_eq!(streamed_n_muon, expected_n_muon);
    assert_eq!(streamed_muon_pt, expected_muon_pt);
    eprintln!(
        "real NanoAOD jagged A/B: compared Events/Muon_pt and Muon_eta vs nMuon for first 32 entries"
    );
}

fn temp_root_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("{}-{name}", std::process::id()));
    path
}
