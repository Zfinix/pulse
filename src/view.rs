//! The live pane: phase, a big readout, a sparkline of recent samples, the
//! phase's progress, and the key hints. Plus the rows a run leaves behind.

use std::time::Instant;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use kiln::keys::{self, Binding};
use kiln::{bigtext, progress, spark, text, theme};

use crate::measure::{HOST, PINGS, Phase};
use crate::stats::{self, Summary};

const FOOTER: [Binding; 2] = [("q", "quit"), ("?", "help")];
const HELP: [Binding; 2] = [
    ("q, esc", "stop the test and quit"),
    ("?", "show or hide this help"),
];
const MAX_WIDTH: usize = 48;
const MARGIN: &str = "  ";

/// What the pane shows: the running phase and the samples it has sent so far.
pub struct Live {
    phase: Option<Phase>,
    started: Instant,
    samples: Vec<f64>,
    help: bool,
}

impl Live {
    /// A pane waiting for the first phase.
    pub fn new() -> Self {
        Self {
            phase: None,
            started: Instant::now(),
            samples: Vec::new(),
            help: false,
        }
    }

    /// Switch to `phase`, dropping the last phase's samples.
    pub fn start(&mut self, phase: Phase) {
        self.phase = Some(phase);
        self.started = Instant::now();
        self.samples.clear();
    }

    /// Add a live reading to the readout and the sparkline.
    pub fn push(&mut self, sample: f64) {
        self.samples.push(sample);
    }

    /// Swap the one-row footer for the full key help, or back.
    pub fn toggle_help(&mut self) {
        self.help = !self.help;
    }

    /// The pane at `width` columns. `colo` is the server location, once known.
    pub fn lines(&self, colo: Option<&str>, width: u16) -> Vec<Line<'static>> {
        let t = theme::get();
        let span = (width as usize)
            .saturating_sub(MARGIN.len() * 2)
            .min(MAX_WIDTH);
        let mut out = vec![Line::default()];

        let title = match self.phase {
            Some(phase) => Span::styled(phase.name(), t.text_style().add_modifier(Modifier::BOLD)),
            None => Span::styled(
                format!("Finding the nearest server on {HOST}"),
                t.dim_style(),
            ),
        };
        let mut head = vec![MARGIN.into(), title];
        if let Some(colo) = colo {
            head.push(Span::styled(format!("  ·  {colo}"), t.faint_style()));
        }
        out.push(Line::from(head));

        if let Some(phase) = self.phase {
            out.push(Line::default());
            let reading = stats::number(self.samples.last().copied().unwrap_or_default());
            let big = bigtext::lines(&reading, t.accent_bold());
            let last = big.len().saturating_sub(1);
            for (row, line) in big.into_iter().enumerate() {
                let mut spans = vec![MARGIN.into()];
                spans.extend(line.spans);
                if row == last {
                    spans.push(Span::styled(format!("  {}", phase.unit()), t.dim_style()));
                }
                out.push(Line::from(spans));
            }
            out.push(Line::default());
            out.push(Line::from(vec![
                MARGIN.into(),
                spark::line(&self.samples, span, t.blue_style()),
            ]));

            let elapsed = self.started.elapsed();
            let (ratio, label) = match phase {
                Phase::Latency => {
                    let count = self.samples.len();
                    (count as f64 / PINGS as f64, format!("{count} of {PINGS}"))
                }
                Phase::Download | Phase::Upload => {
                    let budget = phase.budget();
                    let done = text::elapsed(elapsed.as_secs().min(budget.as_secs()));
                    let total = text::elapsed(budget.as_secs());
                    (
                        elapsed.as_secs_f64() / budget.as_secs_f64(),
                        format!("{done} of {total}"),
                    )
                }
            };
            let mut bar = vec![MARGIN.into()];
            bar.extend(progress::bar(ratio, span));
            bar.push(Span::styled(format!("  {label}"), t.faint_style()));
            out.push(Line::from(bar));
        }

        out.push(Line::default());
        let hints = match self.help {
            true => keys::help(&HELP),
            false => vec![keys::footer(&FOOTER)],
        };
        out.extend(hints.into_iter().map(|line| {
            let mut spans = vec![Span::raw(MARGIN)];
            spans.extend(line.spans);
            Line::from(spans)
        }));
        out
    }
}

/// The last thing a run prints: every reading on one accented row.
pub fn summary(summary: &Summary) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        Line::from(vec![
            MARGIN.into(),
            Span::styled(summary.line(), theme::get().accent_bold()),
        ]),
        Line::default(),
    ]
}
