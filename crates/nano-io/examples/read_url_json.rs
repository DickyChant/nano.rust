use std::error::Error;

#[cfg(feature = "http")]
fn main() -> Result<(), Box<dyn Error>> {
    use nano_core::{BranchSchema, BranchSpec, BranchType};
    use nano_io::events_url_chunked;

    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .ok_or("usage: read_url_json <url> [n] [--insecure]")?;
    let n = args
        .next()
        .as_deref()
        .unwrap_or("10")
        .parse::<usize>()
        .map_err(|err| format!("invalid event count: {err}"))?;
    let insecure = args.any(|arg| arg == "--insecure") || env_flag("NANO_HTTP_INSECURE");

    if insecure {
        std::env::set_var("NANO_HTTP_INSECURE", "1");
    }

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("MET_pt", BranchType::F32),
        BranchSpec::new("run", BranchType::U32),
        BranchSpec::new("event", BranchType::U64),
    ])?;
    let mut events = events_url_chunked(&url, &schema, n.max(1))?;

    print!("[");
    for (index, event) in events.by_ref().take(n).enumerate() {
        let event = event?;
        if index > 0 {
            print!(",");
        }
        print!(
            "{{\"nMuon\":{},\"Muon_pt\":{},\"Muon_eta\":{},\"MET_pt\":{},\"run\":{},\"event\":{}}}",
            event.scalar::<u32>("nMuon")?,
            f32_array_json(&event.vector::<f32>("Muon_pt")?),
            f32_array_json(&event.vector::<f32>("Muon_eta")?),
            finite_f32_json(event.scalar::<f32>("MET_pt")?),
            event.scalar::<u32>("run")?,
            event.scalar::<u64>("event")?,
        );
    }
    println!(
        ",{{\"_meta\":{{\"bytes_fetched\":{},\"file_size\":{}}}}}]",
        events.bytes_fetched(),
        events.file_size()
    );
    Ok(())
}

#[cfg(not(feature = "http"))]
fn main() -> Result<(), Box<dyn Error>> {
    Err("read_url_json requires the nano-io `http` feature".into())
}

#[cfg(feature = "http")]
fn f32_array_json(values: &[f32]) -> String {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&finite_f32_json(*value));
    }
    out.push(']');
    out
}

#[cfg(feature = "http")]
fn finite_f32_json(value: f32) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "null".to_string()
    }
}

#[cfg(feature = "http")]
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
