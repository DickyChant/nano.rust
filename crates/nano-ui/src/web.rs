use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::plot::histogram_svg;
use crate::session::{self, RootInspection, RunSummary, SpecSummary};

pub async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let app = Router::new().route("/", get(dashboard));
    let addr = SocketAddr::from(([127, 0, 0, 1], 7878));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("nano-ui web dashboard: http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct DashboardQuery {
    action: Option<String>,
    spec: Option<String>,
    root: Option<String>,
    input: Option<String>,
    insecure: Option<String>,
    parallel: Option<String>,
}

async fn dashboard(Query(query): Query<DashboardQuery>) -> Html<String> {
    Html(render_dashboard(&query))
}

fn render_dashboard(query: &DashboardQuery) -> String {
    let mut spec_result = String::new();
    let mut root_result = String::new();
    let mut run_result = String::new();

    match query.action.as_deref() {
        Some("validate") => {
            if let Some(path) = query.spec.as_deref().filter(|value| !value.is_empty()) {
                spec_result = match session::open_spec(path) {
                    Ok(summary) => spec_summary_html(&summary),
                    Err(error) => error_html(&error.to_string()),
                };
            }
        }
        Some("inspect") => {
            if let Some(source) = query.root.as_deref().filter(|value| !value.is_empty()) {
                root_result = match session::inspect_root(source, query.insecure.is_some()) {
                    Ok(report) => root_inspection_html(&report),
                    Err(error) => error_html(&error.to_string()),
                };
            }
        }
        Some("run") => {
            if let Some(input) = query.input.as_deref().filter(|value| !value.is_empty()) {
                run_result =
                    match session::run_muon_dag([PathBuf::from(input)], query.parallel.is_some()) {
                        Ok(summary) => run_summary_html(&summary),
                        Err(error) => error_html(&error.to_string()),
                    };
            }
        }
        _ => {}
    }

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>nano-ui</title>
  <style>
    body {{ font-family: system-ui, sans-serif; margin: 0; color: #151515; background: #f7f8fa; }}
    header {{ padding: 18px 24px; background: #1f2937; color: white; }}
    main {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 16px; padding: 16px; }}
    section {{ background: white; border: 1px solid #d6dae1; border-radius: 8px; padding: 16px; min-width: 0; }}
    h1 {{ font-size: 20px; margin: 0; }}
    h2 {{ font-size: 16px; margin: 0 0 12px; }}
    label {{ display: block; font-size: 13px; margin-bottom: 6px; }}
    input[type="text"] {{ width: 100%; box-sizing: border-box; padding: 8px; border: 1px solid #b8c0cc; border-radius: 6px; }}
    button {{ margin-top: 10px; padding: 8px 12px; border: 0; border-radius: 6px; background: #2563eb; color: white; cursor: pointer; }}
    pre {{ white-space: pre-wrap; overflow: auto; background: #f1f3f6; padding: 10px; border-radius: 6px; }}
    .check {{ margin-top: 10px; font-size: 13px; }}
    .error {{ color: #b42318; }}
    .svg-wrap svg {{ max-width: 100%; height: auto; }}
  </style>
</head>
<body>
  <header><h1>nano-ui</h1></header>
  <main>
    <section>
      <h2>Spec</h2>
      <form method="get">
        <input type="hidden" name="action" value="validate">
        <label for="spec">Spec path</label>
        <input id="spec" name="spec" type="text" value="{spec}">
        <button type="submit">Validate</button>
      </form>
      {spec_result}
    </section>
    <section>
      <h2>ROOT Browser</h2>
      <form method="get">
        <input type="hidden" name="action" value="inspect">
        <label for="root">ROOT path or URL</label>
        <input id="root" name="root" type="text" value="{root}">
        <label class="check"><input name="insecure" type="checkbox" {insecure}> Insecure TLS</label>
        <button type="submit">Inspect</button>
      </form>
      {root_result}
    </section>
    <section>
      <h2>Run Muon DAG</h2>
      <form method="get">
        <input type="hidden" name="action" value="run">
        <label for="input">Input ROOT path</label>
        <input id="input" name="input" type="text" value="{input}">
        <label class="check"><input name="parallel" type="checkbox" {parallel}> Parallel</label>
        <button type="submit">Run</button>
      </form>
      {run_result}
    </section>
  </main>
</body>
</html>"#,
        spec = escape(query.spec.as_deref().unwrap_or_default()),
        root = escape(query.root.as_deref().unwrap_or_default()),
        input = escape(query.input.as_deref().unwrap_or_default()),
        insecure = checked(query.insecure.is_some()),
        parallel = checked(query.parallel.is_some()),
    )
}

fn spec_summary_html(summary: &SpecSummary) -> String {
    let branches = summary
        .read_branches
        .iter()
        .map(|branch| format!("{} {}", branch.name, branch.branch_type))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<pre>OK validate {}\nanalysis: {}\nobjects: {}\nregions: {}\noutputs: {}\nread_branches:\n{}</pre>",
        escape(&summary.path.display().to_string()),
        escape(&summary.analysis_name),
        escape(
            &summary
                .objects
                .iter()
                .map(|object| format!("{}:{}", object.name, object.source))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        escape(&summary.regions.join(", ")),
        escape(&summary.outputs.join(", ")),
        escape(&branches)
    )
}

fn root_inspection_html(report: &RootInspection) -> String {
    let mut lines = vec![format!("OK inspect {}", report.source)];
    for tree in &report.trees {
        lines.push(format!("tree {} entries={}", tree.name, tree.entries));
        for branch in &tree.branches {
            lines.push(format!("  {} {}", branch.name, branch.types.join("/")));
        }
    }
    format!("<pre>{}</pre>", escape(&lines.join("\n")))
}

fn run_summary_html(summary: &RunSummary) -> String {
    let svg = histogram_svg(&summary.plot_values)
        .map(|svg| format!("<div class=\"svg-wrap\">{svg}</div>"))
        .unwrap_or_else(|error| error_html(&format!("failed to render SVG histogram: {error}")));
    format!(
        "<pre>OK run ({})\nevents_seen: {}\nevents_selected: {}\nplot_values: {}</pre>{}",
        escape(&summary.mode),
        summary.events_seen,
        summary.events_selected,
        summary.plot_values.len(),
        svg
    )
}

fn error_html(message: &str) -> String {
    format!("<pre class=\"error\">{}</pre>", escape(message))
}

fn checked(value: bool) -> &'static str {
    if value {
        "checked"
    } else {
        ""
    }
}

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
