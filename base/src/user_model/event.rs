use std::sync::{Arc, Mutex};

type Listener = Box<dyn Fn(&Diff) + 'static>;

use super::history::Diff;

/// A simple event emitter that notifies listeners whenever a [`Diff`] is applied.
#[derive(Default)]
pub struct EventEmitter {
    listeners: Arc<Mutex<Vec<Listener>>>,
}

impl EventEmitter {
    /// Creates a new [`EventEmitter`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a listener that will be notified for every [`Diff`] that is applied.
    pub fn subscribe<F>(&mut self, listener: F)
    where
        F: Fn(&Diff) + 'static,
    {
        self.listeners.lock().unwrap().push(Box::new(listener));
    }

    pub(crate) fn emit(&self, diff: &Diff) {
        for cb in self.listeners.lock().unwrap().iter() {
            cb(diff);
        }
    }
}
