use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const KEEP_DAYS: usize = 7;

/// Logs to a daily rolling file (7 days kept), plus stderr in debug builds.
/// Override the level with `RUST_LOG`. Keep the returned guard alive for the
/// life of the app or buffered lines are lost.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    let default_level = if cfg!(debug_assertions) {
        "info,desktop_assistant_lib=debug"
    } else {
        "info"
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    // The appender scans the folder for old files to prune, so it must exist.
    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!("could not create log folder {}: {e}", log_dir.display());
    }

    let file = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("desktop-assistant")
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(log_dir);

    match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_ansi(false).with_writer(writer))
                .with(stderr_layer())
                .init();
            Some(guard)
        }
        Err(e) => {
            // Still log to stderr so a broken log folder doesn't hide everything.
            tracing_subscriber::registry()
                .with(filter)
                .with(stderr_layer())
                .init();
            tracing::warn!("file logging disabled: {e}");
            None
        }
    }
}

/// Console output in debug builds only. A function rather than a shared value
/// because each subscriber stack needs its own layer type.
fn stderr_layer<S>() -> Option<impl tracing_subscriber::Layer<S>>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr))
}
