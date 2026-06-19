use std::path::PathBuf;

use futures::StreamExt;
use nom::number::complete::{be_f32, be_i32, be_u32, be_u64, be_u8};
use root_io::write::{write_tree, Branch};
use root_io::RootFile;

#[tokio::test]
async fn writes_and_reads_scalar_and_jagged_tree() {
    let path = temp_root_path("root_io_scalar_roundtrip.root");
    let _ = std::fs::remove_file(&path);

    let f32_values = vec![1.25_f32, -2.5, 3.75, 8.0];
    let i32_values = vec![-7_i32, 0, 42, 123456];
    let u32_values = vec![0_u32, 1, 4_000_000_000, 17];
    let u64_values = vec![0_u64, 1, 9_000_000_000, u64::MAX - 5];
    let bool_values = vec![true, false, true, true];
    let counts = vec![0_u32, 2, 1, 3];
    let jagged_values = vec![
        Vec::new(),
        vec![10.0_f32, 11.5],
        vec![-1.25],
        vec![2.0, 4.0, 8.0],
    ];

    write_tree(
        &path,
        "Events",
        &[
            Branch::f32("float_branch", f32_values.clone()),
            Branch::i32("int_branch", i32_values.clone()),
            Branch::u32("uint_branch", u32_values.clone()),
            Branch::u64("ulong_branch", u64_values.clone()),
            Branch::bool("bool_branch", bool_values.clone()),
            Branch::u32("nJet", counts.clone()),
            Branch::vec_f32("Jet_pt", jagged_values.clone()),
        ],
    )
    .expect("failed to write ROOT file");

    let file = RootFile::new(path.as_path())
        .await
        .expect("failed to open written ROOT file");
    assert_eq!(file.items().len(), 1);
    let tree = file.items()[0]
        .as_tree()
        .await
        .expect("failed to parse tree");

    let read_f32: Vec<f32> = tree
        .branch_by_name("float_branch")
        .unwrap()
        .as_fixed_size_iterator(|i| be_f32(i))
        .collect()
        .await;
    let read_i32: Vec<i32> = tree
        .branch_by_name("int_branch")
        .unwrap()
        .as_fixed_size_iterator(|i| be_i32(i))
        .collect()
        .await;
    let read_u32: Vec<u32> = tree
        .branch_by_name("uint_branch")
        .unwrap()
        .as_fixed_size_iterator(|i| be_u32(i))
        .collect()
        .await;
    let read_u64: Vec<u64> = tree
        .branch_by_name("ulong_branch")
        .unwrap()
        .as_fixed_size_iterator(|i| be_u64(i))
        .collect()
        .await;
    let read_bool: Vec<bool> = tree
        .branch_by_name("bool_branch")
        .unwrap()
        .as_fixed_size_iterator(|i| be_u8(i).map(|(i, v)| (i, v != 0)))
        .collect()
        .await;
    let read_counts: Vec<u32> = tree
        .branch_by_name("nJet")
        .unwrap()
        .as_fixed_size_iterator(|i| be_u32(i))
        .collect()
        .await;
    let read_jagged: Vec<Vec<f32>> = tree
        .branch_by_name("Jet_pt")
        .unwrap()
        .as_var_size_iterator(|i| be_f32(i), read_counts.clone())
        .collect()
        .await;

    assert_eq!(read_f32, f32_values);
    assert_eq!(read_i32, i32_values);
    assert_eq!(read_u32, u32_values);
    assert_eq!(read_u64, u64_values);
    assert_eq!(read_bool, bool_values);
    assert_eq!(read_counts, counts);
    assert_eq!(read_jagged, jagged_values);
    assert!(file.streamer_infos().await.unwrap().is_empty());

    let _ = std::fs::remove_file(&path);
}

fn temp_root_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("{}-{name}", std::process::id()));
    path
}
