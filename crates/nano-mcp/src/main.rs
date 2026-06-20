use std::io::{self, BufRead, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    if let Err(error) = run_stdio() {
        eprintln!("nano-mcp stdio error: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = nano_mcp::handle_json_rpc_line(&line) {
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }

    Ok(())
}
