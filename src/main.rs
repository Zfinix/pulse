//! pulse: internet speed in your terminal. Parses flags, starts the network
//! worker, and either watches it in a live pane or prints the result.

mod measure;
mod stats;
mod view;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyModifiers};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use kiln::cells;
use kiln::guard::TuiGuard;
use kiln::render::Renderable;
use kiln::terminal::{Tui, TuiEvent, restore_raw};
use kiln::theme;

use measure::{Event, Failure, Phase};
use stats::Summary;

const TICK: Duration = Duration::from_millis(100);
const USAGE: &str = "pulse: internet speed in your terminal

Usage: pulse [options]

Options:
  --no-download    skip the download test
  --no-upload      skip the upload test
  --json           print one JSON object and nothing else
  -q, --quiet      print only the summary line
  --theme <name>   colours for the live pane (default ocean, or PULSE_THEME)
  --themes         list the themes
  -h, --help       show this help
  -V, --version    show the version";

/// Where the result goes.
enum Output {
    Live,
    Quiet,
    Json,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pulse: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let mut theme_name = std::env::var("PULSE_THEME").unwrap_or_else(|_| "ocean".into());
    let mut output = Output::Live;
    let mut phases = vec![Phase::Latency, Phase::Download, Phase::Upload];
    let names = || {
        theme::all()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-download" => phases.retain(|phase| *phase != Phase::Download),
            "--no-upload" => phases.retain(|phase| *phase != Phase::Upload),
            "--json" => output = Output::Json,
            "-q" | "--quiet" => output = Output::Quiet,
            "--theme" => {
                let Some(name) = args.next() else {
                    bail!("--theme needs a name, try one of: {}", names().join(", "));
                };
                theme_name = name;
            }
            "--themes" => {
                println!("{}", names().join("\n"));
                return Ok(ExitCode::SUCCESS);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            "-V" | "--version" => {
                println!("pulse {}", env!("CARGO_PKG_VERSION"));
                return Ok(ExitCode::SUCCESS);
            }
            other => bail!("there is no {other} option, run pulse --help to see them all"),
        }
    }

    let Some(entry) = theme::named(&theme_name) else {
        bail!(
            "there is no theme called {theme_name}, try one of: {}",
            names().join(", ")
        );
    };
    theme::set(entry.theme);
    theme::settle();

    let (tx, mut rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || measure::run(&phases, tx));

    if matches!(output, Output::Live) && std::io::stdout().is_terminal() {
        return live(rx).await;
    }
    let mut summary = Summary::default();
    while let Some(event) = rx.recv().await {
        record(&mut summary, &event);
        match event {
            Event::Failed(Failure { message, detail }) => {
                eprintln!("{message}\n{detail}");
                return Ok(ExitCode::FAILURE);
            }
            Event::Finished => break,
            Event::Location(_)
            | Event::Started(_)
            | Event::Sample(_)
            | Event::Latency { .. }
            | Event::Download(_)
            | Event::Upload(_) => {}
        }
    }
    match output {
        Output::Json => println!("{}", summary.json()),
        Output::Live | Output::Quiet => println!("{}", summary.line()),
    }
    Ok(ExitCode::SUCCESS)
}

async fn live(mut rx: UnboundedReceiver<Event>) -> Result<ExitCode> {
    let _guard = TuiGuard::install(restore_raw);
    let mut tui = Tui::new(12)?;
    let frames = tui.frame_requester();
    let mut pane = view::Live::new();
    let mut summary = Summary::default();

    loop {
        let lines = pane.lines(summary.colo.as_deref(), tui.width());
        tui.draw(lines.desired_height(tui.width()), |frame| {
            lines.render(frame.area(), frame.buffer_mut());
        })?;
        frames.schedule_in(TICK);

        tokio::select! {
            input = tui.next_event() => match input {
                TuiEvent::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(ExitCode::SUCCESS),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(ExitCode::SUCCESS);
                    }
                    KeyCode::Char('?') => pane.toggle_help(),
                    _ => {}
                },
                TuiEvent::Resize => tui.resized()?,
                TuiEvent::Mouse(_) | TuiEvent::Paste(_) | TuiEvent::Draw => {}
            },
            message = rx.recv() => {
                let Some(event) = message else {
                    return Ok(ExitCode::FAILURE);
                };
                record(&mut summary, &event);
                let width = tui.width() as usize;
                let (latency, download, upload) =
                    (Phase::Latency.name(), Phase::Download.name(), Phase::Upload.name());
                let row = match event {
                    Event::Location(_) => None,
                    Event::Started(phase) => {
                        pane.start(phase);
                        None
                    }
                    Event::Sample(sample) => {
                        pane.push(sample);
                        None
                    }
                    Event::Latency { ping_ms, jitter_ms } => {
                        let (ping, jitter) = (stats::number(ping_ms), stats::number(jitter_ms));
                        Some(format!("{latency:<10}ping {ping} ms, jitter {jitter} ms"))
                    }
                    Event::Download(mbps) => {
                        Some(format!("{download:<10}{} Mbps", stats::number(mbps)))
                    }
                    Event::Upload(mbps) => Some(format!("{upload:<10}{} Mbps", stats::number(mbps))),
                    Event::Failed(Failure { message, detail }) => {
                        tui.insert_history(cells::error_box(&[message.to_string()], width))?;
                        tui.insert_history(cells::notice(&detail, width))?;
                        return Ok(ExitCode::FAILURE);
                    }
                    Event::Finished => {
                        tui.insert_history(view::summary(&summary))?;
                        return Ok(ExitCode::SUCCESS);
                    }
                };
                if let Some(row) = row {
                    tui.insert_history(cells::notice(&row, width))?;
                }
            }
        }
    }
}

fn record(summary: &mut Summary, event: &Event) {
    match event {
        Event::Location(colo) => summary.colo = Some(colo.clone()),
        Event::Latency { ping_ms, jitter_ms } => {
            summary.ping_ms = Some(*ping_ms);
            summary.jitter_ms = Some(*jitter_ms);
        }
        Event::Download(mbps) => summary.download_mbps = Some(*mbps),
        Event::Upload(mbps) => summary.upload_mbps = Some(*mbps),
        Event::Started(_) | Event::Sample(_) | Event::Failed(_) | Event::Finished => {}
    }
}
