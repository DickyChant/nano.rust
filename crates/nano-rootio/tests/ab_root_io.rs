use std::path::{Path, PathBuf};

use futures::executor::block_on;
use futures::StreamExt;
use nano_rootio::RootFile;

type NomResult<'a, T> = nom::IResult<&'a [u8], T, nom::error::Error<&'a [u8]>>;

fn be_i32(input: &[u8]) -> NomResult<'_, i32> {
    nom::number::complete::be_i32(input)
}

fn be_i64(input: &[u8]) -> NomResult<'_, i64> {
    nom::number::complete::be_i64(input)
}

fn be_u32(input: &[u8]) -> NomResult<'_, u32> {
    nom::number::complete::be_u32(input)
}

fn be_u64(input: &[u8]) -> NomResult<'_, u64> {
    nom::number::complete::be_u64(input)
}

fn be_f32(input: &[u8]) -> NomResult<'_, f32> {
    nom::number::complete::be_f32(input)
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
