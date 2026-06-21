use std::process::ExitCode;

#[cfg(feature = "web")]
#[tokio::main]
async fn main() -> ExitCode {
    use std::error::Error;

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    let result: Result<(), Box<dyn Error>> = if args.iter().any(|arg| arg == "--tui") {
        nano_ui::tui::run().map_err(Box::<dyn Error>::from)
    } else {
        nano_ui::web::run().await
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "web"))]
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--web") {
        eprintln!("rebuild with --features web");
        return ExitCode::FAILURE;
    }
    match nano_ui::tui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("usage: nano-ui [--web] [--tui]\n\nDefault: TUI. With --features web, runs the local dashboard unless --tui is passed.");
}
