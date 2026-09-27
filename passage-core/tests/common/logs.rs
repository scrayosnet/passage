//! What the driver logged, so a test can assert on what an operator would see.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// Everything recorded, as `(level, "message field=value ...")`.
///
/// This is how the server's own reporting is tested: a connection the accept loop owns reports by
/// logging and by nothing else, so the thing under test *is* the log line.
#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    /// Every event recorded so far.
    pub fn events(&self) -> Vec<(Level, String)> {
        self.0.lock().expect("not poisoned").clone()
    }

    /// How many events mention `needle`.
    pub fn count(&self, needle: &str) -> usize {
        self.events()
            .iter()
            .filter(|(_, text)| text.contains(needle))
            .count()
    }

    /// Every event whose text contains `needle`.
    pub fn all(&self, needle: &str) -> Vec<(Level, String)> {
        self.events()
            .into_iter()
            .filter(|(_, text)| text.contains(needle))
            .collect()
    }

    /// The one event whose text contains `needle`, and its level.
    ///
    /// Panics unless exactly one matches, so a test cannot quietly assert on the wrong line.
    pub fn find(&self, needle: &str) -> (Level, String) {
        let events = self.events();
        let mut matching = events.iter().filter(|(_, text)| text.contains(needle));
        let found = matching
            .next()
            .unwrap_or_else(|| panic!("no event mentioning {needle:?} in {events:#?}"))
            .clone();
        assert!(
            matching.next().is_none(),
            "more than one event mentioning {needle:?} in {events:#?}",
        );
        found
    }
}

/// Records everything logged for as long as the guard is held.
///
/// Thread-local rather than global, which is what lets every test have its own: `#[tokio::test]`
/// runs on a current-thread runtime, so the connection tasks the server spawns land on this same
/// thread and see it.
pub fn record_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(Recorder(logs.clone()));
    let guard = tracing::subscriber::set_default(subscriber);
    (logs, guard)
}

struct Recorder(Logs);

impl<S: Subscriber + for<'a> LookupSpan<'a>> tracing_subscriber::Layer<S> for Recorder {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut text = Flatten(String::new());
        event.record(&mut text);
        self.0
            .0
            .lock()
            .expect("not poisoned")
            .push((*event.metadata().level(), text.0));
    }
}

/// Renders an event as one string, so assertions can be about what it says rather than about how
/// tracing structures it.
struct Flatten(String);

impl Visit for Flatten {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?} ");
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}
