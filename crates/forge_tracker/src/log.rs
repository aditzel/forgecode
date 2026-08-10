use std::path::PathBuf;

use tracing::debug;
use tracing_appender::non_blocking::{self, WorkerGuard};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{self, Layer, filter};

use crate::Tracker;
use crate::can_track::can_track;

pub fn init_tracing(log_path: PathBuf, tracker: Tracker) -> anyhow::Result<Guard> {
    debug!(path = %log_path.display(), "Initializing logging system in JSON format");

    let (writer, guard, level) = prepare_writer(log_path);

    // Create a filter that only allows logs from forge_ modules
    let filter = filter::filter_fn(|metadata| metadata.target().starts_with("forge_"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_thread_ids(false)
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .with_writer(writer)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_env("FORGE_LOG").unwrap_or(level))
        .with(fmt_layer)
        .with(PosthogErrorLayer::new(tracker))
        .init();

    Ok(Guard(guard))
}

/// Logs always go to a local rolling file; errors are tracked separately via
/// explicit EventKind::Error dispatches. When tracking is enabled the default
/// level is info; otherwise debug for local development.
fn prepare_writer(
    log_path: PathBuf,
) -> (
    non_blocking::NonBlocking,
    WorkerGuard,
    tracing_subscriber::EnvFilter,
) {
    let append = tracing_appender::rolling::daily(log_path, "forge.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(append);
    let env = if can_track() {
        tracing_subscriber::EnvFilter::new("forge=info")
    } else {
        tracing_subscriber::EnvFilter::new("forge=debug")
    };
    (non_blocking, guard, env)
}

pub struct Guard(#[allow(dead_code)] WorkerGuard);

/// A tracing Layer that forwards error-level events from forge_ modules to the
/// tracker as EventKind::Error. This is the single pipeline through which
/// errors reach PostHog; lower-level crates just use `tracing::error!`.
struct PosthogErrorLayer {
    tracker: Tracker,
    runtime: tokio::runtime::Runtime,
}

impl PosthogErrorLayer {
    fn new(tracker: Tracker) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(1)
            .build()
            .expect("Failed to create Tokio runtime");
        Self { tracker, runtime }
    }
}

impl<S: tracing::Subscriber> Layer<S> for PosthogErrorLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        if *metadata.level() != tracing::Level::ERROR
            || !metadata.target().starts_with("forge_")
        {
            return;
        }

        // Render the event's fields (message + structured fields) into a
        // single string.
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let mut message = visitor.0;
        if let (Some(file), Some(line)) = (metadata.file(), metadata.line()) {
            message = format!("{file}:{line} {message}");
        }

        let tracker = self.tracker.clone();
        self.runtime.spawn(async move {
            let _ = tracker.dispatch(crate::EventKind::Error(message)).await;
        });
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        } else {
            self.0.push_str(&format!("{}={value:?}", field.name()));
        }
    }
}
