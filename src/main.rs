//! pulse: internet speed in your terminal. Parses flags, starts the network
//! worker, and prints each result as it lands.

mod measure;
mod stats;

use std::process::ExitCode;

use anyhow::{Result, bail};
use tokio::sync::mpsc;

use measure::{Event, Failure, Phase};
use stats::Summary;

const USAGE: &str = "pulse: internet speed in your terminal

Usage: pulse [options]

Options:
  -h, --help       show this help
  -V, --version    show the version";

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
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
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

    let (tx, mut rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || measure::run(tx));

    let mut summary = Summary::default();
    let (latency, download) = (Phase::Latency.name(), Phase::Download.name());
    while let Some(event) = rx.recv().await {
        record(&mut summary, &event);
        match event {
            Event::Latency { ping_ms, jitter_ms } => {
                let (ping, jitter) = (stats::number(ping_ms), stats::number(jitter_ms));
                println!("{latency:<10}ping {ping} ms, jitter {jitter} ms");
            }
            Event::Download(mbps) => println!("{download:<10}{} Mbps", stats::number(mbps)),
            Event::Failed(Failure { message, detail }) => {
                eprintln!("{message}\n{detail}");
                return Ok(ExitCode::FAILURE);
            }
            Event::Finished => break,
        }
    }
    println!("{}", summary.line());
    Ok(ExitCode::SUCCESS)
}

fn record(summary: &mut Summary, event: &Event) {
    match event {
        Event::Latency { ping_ms, jitter_ms } => {
            summary.ping_ms = Some(*ping_ms);
            summary.jitter_ms = Some(*jitter_ms);
        }
        Event::Download(mbps) => summary.download_mbps = Some(*mbps),
        Event::Failed(_) | Event::Finished => {}
    }
}
