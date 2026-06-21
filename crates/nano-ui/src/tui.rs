use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::plot::ascii_histogram;
use crate::session::{self, RootInspection, RunSummary, SpecSummary};

pub fn run() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = App::default().run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ActivePane {
    #[default]
    Root,
    Spec,
    Run,
}

impl ActivePane {
    fn next(self) -> Self {
        match self {
            Self::Root => Self::Spec,
            Self::Spec => Self::Run,
            Self::Run => Self::Root,
        }
    }
}

#[derive(Debug, Default)]
struct App {
    active: ActivePane,
    root_path: String,
    root_insecure: bool,
    root_output: String,
    spec_path: String,
    spec_output: String,
    run_input: String,
    run_parallel: bool,
    run_output: String,
}

impl App {
    fn run(mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;
            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if self.handle_key(key) {
                break;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Tab => self.active = self.active.next(),
            KeyCode::Backspace => {
                self.active_input().pop();
            }
            KeyCode::Enter => self.run_active_action(),
            KeyCode::Char('k') if self.active == ActivePane::Spec => self.show_kernel(),
            KeyCode::Char('i') if self.active == ActivePane::Root => {
                self.root_insecure = !self.root_insecure;
            }
            KeyCode::Char('p') if self.active == ActivePane::Run => {
                self.run_parallel = !self.run_parallel;
            }
            KeyCode::Char(value) => self.active_input().push(value),
            _ => {}
        }
        false
    }

    fn active_input(&mut self) -> &mut String {
        match self.active {
            ActivePane::Root => &mut self.root_path,
            ActivePane::Spec => &mut self.spec_path,
            ActivePane::Run => &mut self.run_input,
        }
    }

    fn run_active_action(&mut self) {
        match self.active {
            ActivePane::Root => {
                self.root_output = match session::inspect_root(&self.root_path, self.root_insecure)
                {
                    Ok(report) => format_root_inspection(&report),
                    Err(error) => error.to_string(),
                };
            }
            ActivePane::Spec => {
                self.spec_output = match session::open_spec(&self.spec_path) {
                    Ok(summary) => format_spec_summary(&summary),
                    Err(error) => error.to_string(),
                };
            }
            ActivePane::Run => {
                let input = PathBuf::from(self.run_input.trim());
                self.run_output = match session::run_muon_dag([input], self.run_parallel) {
                    Ok(summary) => format_run_summary(&summary),
                    Err(error) => error.to_string(),
                };
            }
        }
    }

    fn show_kernel(&mut self) {
        self.spec_output = match session::codegen_source(&self.spec_path) {
            Ok(source) => source,
            Err(error) => error.to_string(),
        };
    }

    fn render(&self, frame: &mut Frame<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(frame.size());

        let help = "Tab: pane  Enter: run pane  q/Esc: quit  root: i toggles insecure  spec: k shows kernel  run: p toggles parallel";
        frame.render_widget(Paragraph::new(help), chunks[0]);
        frame.render_widget(self.root_pane(), chunks[1]);
        frame.render_widget(self.spec_pane(), chunks[2]);
        frame.render_widget(self.run_pane(), chunks[3]);
    }

    fn root_pane(&self) -> Paragraph<'_> {
        let body = format!(
            "path/url: {}\ninsecure TLS: {}\n\n{}",
            self.root_path,
            if self.root_insecure { "on" } else { "off" },
            self.root_output
        );
        pane("ROOT Browser", self.active == ActivePane::Root, body)
    }

    fn spec_pane(&self) -> Paragraph<'_> {
        let body = format!("spec path: {}\n\n{}", self.spec_path, self.spec_output);
        pane("Spec", self.active == ActivePane::Spec, body)
    }

    fn run_pane(&self) -> Paragraph<'_> {
        let body = format!(
            "input root: {}\nparallel: {}\n\n{}",
            self.run_input,
            if self.run_parallel { "on" } else { "off" },
            self.run_output
        );
        pane("Run Muon DAG", self.active == ActivePane::Run, body)
    }
}

fn pane(title: &'static str, active: bool, body: String) -> Paragraph<'static> {
    let border_style = if active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Paragraph::new(Text::from(body))
        .block(
            Block::default()
                .title(Line::from(title))
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .wrap(Wrap { trim: false })
}

fn format_spec_summary(summary: &SpecSummary) -> String {
    let objects = summary
        .objects
        .iter()
        .map(|object| format!("{}:{}", object.name, object.source))
        .collect::<Vec<_>>()
        .join(", ");
    let read_branches = summary
        .read_branches
        .iter()
        .map(|branch| format!("{} {}", branch.name, branch.branch_type))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "OK validate {}\nanalysis: {}\nobjects: {}\nregions: {}\noutputs: {}\nread_branches: {}",
        summary.path.display(),
        summary.analysis_name,
        objects,
        summary.regions.join(", "),
        summary.outputs.join(", "),
        read_branches
    )
}

fn format_root_inspection(report: &RootInspection) -> String {
    let mut lines = vec![format!("OK inspect {}", report.source)];
    for tree in &report.trees {
        lines.push(format!("tree {} entries={}", tree.name, tree.entries));
        for branch in &tree.branches {
            lines.push(format!("  {} {}", branch.name, branch.types.join("/")));
        }
    }
    lines.join("\n")
}

fn format_run_summary(summary: &RunSummary) -> String {
    format!(
        "OK run ({})\nevents_seen: {}\nevents_selected: {}\n\nlead_muon_pt_histogram:\n{}",
        summary.mode,
        summary.events_seen,
        summary.events_selected,
        ascii_histogram(&summary.plot_values, 10, None)
    )
}
