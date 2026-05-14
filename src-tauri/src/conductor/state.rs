//! Conductor's state, split by access pattern.
//!
//! Different state has different access shapes, so a single coarse mutex
//! makes everyone wait on everyone else. Here each piece gets exactly the
//! synchronization it needs:
//!
//! - `project: ArcSwap<ProjectRow>` — rarely mutated (rename), heavily read.
//!   Swaps are lock-free; reads load an Arc snapshot.
//! - `state: watch::Sender<ConductorState>` — atomic with a built-in
//!   subscribe channel, ready for reactive UI without polling.
//! - `wrap_up_requested: AtomicBool` — single-bit flag, lock-free.
//! - `waker: Mutex<Option<oneshot::Sender>>` — exactly one in flight when
//!   parked in `Presenting`; cheap, rare.
//! - `questions: Mutex<HashMap<String, oneshot::Sender<String>>>` — rare
//!   writes, never read on a hot path.
//!
//! There is no `stopped` flag — cancellation lives in the `CancellationToken`
//! and is the single source of truth.

use crate::types::{ConductorState, ProjectRow};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, watch};

pub(super) struct Inner {
    pub project: ArcSwap<ProjectRow>,
    pub state: watch::Sender<ConductorState>,
    pub wrap_up_requested: AtomicBool,
    pub waker: Mutex<Option<oneshot::Sender<()>>>,
    pub questions: Mutex<HashMap<String, oneshot::Sender<String>>>,
    /// Set while `run_loop` is sleeping out a provider rate-limit. The
    /// snapshot reads this so the UI can draw a countdown; clearing it
    /// is the cooldown-finished signal.
    pub cooldown: Mutex<Option<super::cooldown::CooldownInfo>>,
    /// Wake the cooldown sleep early. Filled by `run_loop` right before
    /// it sleeps; consumed when the user presses Continue mid-cooldown
    /// instead of waiting out the timer.
    pub cooldown_skip: Mutex<Option<oneshot::Sender<()>>>,
}

impl Inner {
    pub fn new(project: ProjectRow) -> Self {
        let (state, _rx) = watch::channel(ConductorState::Idle);
        Self {
            project: ArcSwap::from_pointee(project),
            state,
            wrap_up_requested: AtomicBool::new(false),
            waker: Mutex::new(None),
            questions: Mutex::new(HashMap::new()),
            cooldown: Mutex::new(None),
            cooldown_skip: Mutex::new(None),
        }
    }

    /// Cheap read snapshot of the current project. Cloning the `Arc` is free.
    pub fn project_snapshot(&self) -> Arc<ProjectRow> {
        self.project.load_full()
    }
}
