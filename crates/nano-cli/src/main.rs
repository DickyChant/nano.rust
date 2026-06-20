use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let options = nano_cli::parse_options(&args);

    match nano_cli::run(args) {
        Ok(output) => {
            let rendered = if options.json {
                nano_cli::render_json_output(&output).expect("serialize CLI output")
            } else {
                nano_cli::render_text(&output)
            };
            println!("{rendered}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let rendered = if options.json {
                nano_cli::render_json_error(&error).expect("serialize CLI error")
            } else {
                nano_cli::render_text_error(&error)
            };
            eprintln!("{rendered}");
            ExitCode::FAILURE
        }
    }
}
