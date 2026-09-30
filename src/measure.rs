//! The network side. A run happens on a worker thread against Cloudflare's
//! public speed test and reports each result over a channel.

use std::io::{self, Read};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;
use ureq::{Agent, SendBody};

use crate::stats;

/// Round trips timed in the latency phase, after one warm-up.
pub const PINGS: usize = 10;

const DOWN_URL: &str = "https://speed.cloudflare.com/__down";
const UP_URL: &str = "https://speed.cloudflare.com/__up";
const TRACE_URL: &str = "https://speed.cloudflare.com/cdn-cgi/trace";

const FIRST_CHUNK: u64 = 100_000;
const MAX_CHUNK: u64 = 25_000_000;
const CHUNK_TIME: Duration = Duration::from_secs(1);
const DOWN_CAP: u64 = 150_000_000;
const UP_CAP: u64 = 50_000_000;
const READ_BUF: usize = 64 * 1024;

/// One stage of a run, in the order they happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Latency,
    Download,
    Upload,
}

impl Phase {
    /// The name shown next to the phase's result.
    pub fn name(self) -> &'static str {
        match self {
            Phase::Latency => "Latency",
            Phase::Download => "Download",
            Phase::Upload => "Upload",
        }
    }

    /// How long the phase is allowed to run. Latency also stops at [`PINGS`].
    pub fn budget(self) -> Duration {
        match self {
            Phase::Latency => Duration::from_secs(5),
            Phase::Download => Duration::from_secs(8),
            Phase::Upload => Duration::from_secs(6),
        }
    }
}

/// Something the worker wants the UI to know.
#[derive(Debug)]
pub enum Event {
    /// The Cloudflare data centre answering, e.g. `LHR`.
    Location(String),
    /// The latency phase finished: median ping and jitter, in milliseconds.
    Latency { ping_ms: f64, jitter_ms: f64 },
    /// The download phase finished, in Mbps.
    Download(f64),
    /// The upload phase finished, in Mbps.
    Upload(f64),
    /// A request failed and the run stopped.
    Failed(Failure),
    /// Every phase finished.
    Finished,
}

/// A failed run: one plain sentence for people and the raw error under it.
#[derive(Debug)]
pub struct Failure {
    /// What happened and what to do next, in one sentence.
    pub message: &'static str,
    /// The error as the HTTP client reported it.
    pub detail: String,
}

impl From<ureq::Error> for Failure {
    fn from(error: ureq::Error) -> Self {
        let message = match &error {
            ureq::Error::StatusCode(_) => {
                "speed.cloudflare.com turned the test away. Wait a minute and try again."
            }
            ureq::Error::Timeout(_) => {
                "speed.cloudflare.com stopped answering. Check your connection and try again."
            }
            _ => "Could not reach speed.cloudflare.com. Check your connection and try again.",
        };
        Self {
            message,
            detail: error.to_string(),
        }
    }
}

/// Run every phase in order, then send [`Event::Finished`], or
/// [`Event::Failed`] at the first error. Blocks, so call it from its own thread.
pub fn run(events: UnboundedSender<Event>) {
    let agent: Agent = Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_recv_response(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(15)))
        .build()
        .new_agent();

    let trace = agent
        .get(TRACE_URL)
        .config()
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .call()
        .and_then(|mut response| response.body_mut().read_to_string());
    if let Some(colo) = trace.ok().and_then(|body| {
        body.lines()
            .find_map(|l| l.strip_prefix("colo=").map(str::to_string))
    }) {
        let _ = events.send(Event::Location(colo));
    }

    for phase in [Phase::Latency, Phase::Download, Phase::Upload] {
        let outcome = match phase {
            Phase::Latency => latency(&agent),
            Phase::Download => download(&agent).map(Event::Download),
            Phase::Upload => upload(&agent).map(Event::Upload),
        };
        match outcome {
            Ok(event) => {
                let _ = events.send(event);
            }
            Err(error) => {
                let _ = events.send(Event::Failed(error.into()));
                return;
            }
        }
    }
    let _ = events.send(Event::Finished);
}

fn latency(agent: &Agent) -> Result<Event, ureq::Error> {
    let deadline = Instant::now() + Phase::Latency.budget();
    let url = format!("{DOWN_URL}?bytes=0");
    agent.get(&url).call()?.body_mut().read_to_vec()?;
    let mut pings = Vec::with_capacity(PINGS);
    while pings.len() < PINGS && Instant::now() < deadline {
        let start = Instant::now();
        let mut response = agent.get(&url).call()?;
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        response.body_mut().read_to_vec()?;
        pings.push(ms);
    }
    Ok(Event::Latency {
        ping_ms: stats::median(&pings).unwrap_or_default(),
        jitter_ms: stats::jitter(&pings).unwrap_or_default(),
    })
}

fn download(agent: &Agent) -> Result<f64, ureq::Error> {
    let deadline = Instant::now() + Phase::Download.budget();
    let mut meter = Meter::new();
    let mut buf = vec![0; READ_BUF];
    let mut chunk = FIRST_CHUNK;
    while Instant::now() < deadline && meter.total < DOWN_CAP {
        let sent = Instant::now();
        let mut response = agent.get(&format!("{DOWN_URL}?bytes={chunk}")).call()?;
        let mut body = response.body_mut().as_reader();
        loop {
            let read = body.read(&mut buf)?;
            if read == 0 {
                break;
            }
            meter.add(read as u64);
            if Instant::now() >= deadline {
                break;
            }
        }
        meter.warmed();
        chunk = next_chunk(chunk, sent.elapsed(), DOWN_CAP.saturating_sub(meter.total));
    }
    Ok(meter.mbps())
}

fn upload(agent: &Agent) -> Result<f64, ureq::Error> {
    let deadline = Instant::now() + Phase::Upload.budget();
    let mut meter = Meter::new();
    let mut chunk = FIRST_CHUNK;
    while Instant::now() < deadline && meter.total < UP_CAP {
        let sent = Instant::now();
        let mut payload = Payload {
            left: chunk,
            meter: &mut meter,
        };
        agent
            .post(UP_URL)
            .header("content-type", "application/octet-stream")
            .header("content-length", chunk)
            .send(SendBody::from_reader(&mut payload))?
            .body_mut()
            .read_to_vec()?;
        meter.warmed();
        chunk = next_chunk(chunk, sent.elapsed(), UP_CAP.saturating_sub(meter.total));
    }
    Ok(meter.mbps())
}

/// Scale the last request toward [`CHUNK_TIME`], at most 4x, never smaller,
/// so fast links get big requests and slow ones never overrun the budget much.
fn next_chunk(last: u64, took: Duration, left: u64) -> u64 {
    let scale = (CHUNK_TIME.as_secs_f64() / took.as_secs_f64().max(0.001)).clamp(1.0, 4.0);
    ((last as f64 * scale) as u64).min(MAX_CHUNK).min(left)
}

/// Counts bytes moved, and gives a result that leaves out the first request,
/// which only warms the connection up.
struct Meter {
    total: u64,
    counted: u64,
    counting_since: Option<Instant>,
    started: Instant,
}

impl Meter {
    fn new() -> Self {
        Self {
            total: 0,
            counted: 0,
            counting_since: None,
            started: Instant::now(),
        }
    }

    fn add(&mut self, bytes: u64) {
        self.total += bytes;
        if self.counting_since.is_some() {
            self.counted += bytes;
        }
    }

    fn warmed(&mut self) {
        self.counting_since.get_or_insert_with(Instant::now);
    }

    fn mbps(&self) -> f64 {
        match self.counting_since {
            Some(since) if self.counted > 0 => stats::mbps(self.counted, since.elapsed()),
            _ => stats::mbps(self.total, self.started.elapsed()),
        }
    }
}

/// An upload body of `left` filler bytes that counts itself as it is sent.
struct Payload<'m> {
    left: u64,
    meter: &'m mut Meter,
}

impl Read for Payload<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        buf[..n].fill(b'0');
        self.left -= n as u64;
        self.meter.add(n as u64);
        Ok(n)
    }
}
