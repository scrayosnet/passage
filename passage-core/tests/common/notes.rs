//! The per-connection state the tests use.

use std::sync::{Arc, Mutex};

/// What one side of a connection saw, in order.
///
/// Cheap to clone, and every clone reads the same list: a test can hold one while the connection
/// still owns another. That is what lets a state factory hand out notes the test can read *while*
/// the server is running, rather than only after it has ended.
#[derive(Clone, Default, Debug)]
pub struct Notes(Arc<Mutex<Vec<String>>>);

impl Notes {
    /// Records one line.
    pub fn push(&self, line: impl Into<String>) {
        self.0.lock().expect("not poisoned").push(line.into());
    }

    /// Everything recorded so far.
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().expect("not poisoned").clone()
    }

    /// How many lines contain `needle`.
    pub fn count_lines(&self, needle: &str) -> usize {
        self.lines()
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }

    /// Whether any line contains `needle`.
    pub fn saw(&self, needle: &str) -> bool {
        self.lines().iter().any(|line| line.contains(needle))
    }

    /// The one line containing `needle`.
    ///
    /// Panics unless exactly one matches, so a test cannot quietly assert on the wrong line.
    pub fn find(&self, needle: &str) -> String {
        let lines = self.lines();
        let mut matching = lines.iter().filter(|line| line.contains(needle));
        let found = matching
            .next()
            .unwrap_or_else(|| panic!("nothing mentioning {needle:?} in {lines:#?}"))
            .clone();
        assert!(
            matching.next().is_none(),
            "more than one line mentioning {needle:?} in {lines:#?}",
        );
        found
    }
}
