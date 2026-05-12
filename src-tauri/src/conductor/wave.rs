//! Specialist DAG runner. Walks the task graph in waves up to MAX_CONCURRENCY
//! at a time, cascade-skipping dependents of failed/skipped tasks.

use super::outputs::ArchTask;
use super::Conductor;
use crate::events::EventPayload;
use crate::types::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Upper bound on specialists running in parallel within a single wave.
///
/// Claude Code itself has no request-rate limit (only a token budget), so
/// the practical ceiling here is the user's RAM (~30-50MB per `claude`
/// subprocess) and how much money they're willing to spend per wave. The
/// integration phase (rebase + ff-merge) is still serialized through the
/// merge lock, so high parallelism doesn't break git.
const MAX_CONCURRENCY: usize = 50;

/// Internal book-keeping for wave scheduling. Holding all the per-id sets in
/// one struct keeps `execute_specialist_waves` readable.
struct Scheduler {
    by_id: HashMap<String, ArchTask>,
    task_rows: HashMap<String, TaskRow>,
    completed: HashSet<String>,
    failed: HashSet<String>,
    skipped: HashSet<String>,
    remaining: HashSet<String>,
}

impl Scheduler {
    fn from_store(
        tasks: &[ArchTask],
        task_rows: HashMap<String, TaskRow>,
    ) -> Self {
        let by_id: HashMap<String, ArchTask> =
            tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let mut completed = HashSet::new();
        let mut failed = HashSet::new();
        let mut skipped = HashSet::new();
        let mut remaining = HashSet::new();
        for t in tasks {
            match task_rows.get(&t.id).map(|r| r.status) {
                Some(TaskStatus::Done) => {
                    completed.insert(t.id.clone());
                }
                Some(TaskStatus::Failed) => {
                    failed.insert(t.id.clone());
                }
                Some(TaskStatus::Skipped) => {
                    skipped.insert(t.id.clone());
                }
                _ => {
                    remaining.insert(t.id.clone());
                }
            }
        }
        Self {
            by_id,
            task_rows,
            completed,
            failed,
            skipped,
            remaining,
        }
    }

    /// Pick up to MAX_CONCURRENCY tasks whose dependencies are all done; also
    /// emits a `skip_now` list for tasks whose dependencies have failed.
    fn next_wave(&self) -> (Vec<ArchTask>, Vec<String>) {
        let mut ready = Vec::<ArchTask>::new();
        let mut skip_now = Vec::<String>::new();
        for id in &self.remaining {
            let Some(t) = self.by_id.get(id) else {
                tracing::warn!(task_id = %id, "wave: id missing from by_id, dropping");
                skip_now.push(id.clone());
                continue;
            };
            let deps = &t.depends_on;
            if deps
                .iter()
                .any(|d| self.failed.contains(d) || self.skipped.contains(d))
            {
                skip_now.push(id.clone());
                continue;
            }
            if deps
                .iter()
                .all(|d| self.completed.contains(d) || !self.by_id.contains_key(d))
            {
                ready.push(t.clone());
            }
        }
        ready.truncate(MAX_CONCURRENCY);
        (ready, skip_now)
    }
}

impl Conductor {
    pub(super) async fn execute_specialist_waves(
        self: Arc<Self>,
        tasks: &[ArchTask],
        iter: &IterationRow,
        project: &ProjectRow,
    ) {
        let mut task_rows: HashMap<String, TaskRow> = HashMap::new();
        for r in self.store.iteration_tasks(&iter.id) {
            if let Some(aid) = &r.architect_id {
                task_rows.insert(aid.clone(), r);
            }
        }
        let mut sched = Scheduler::from_store(tasks, task_rows);
        tracing::info!(
            iteration = iter.number,
            total = tasks.len(),
            remaining = sched.remaining.len(),
            completed = sched.completed.len(),
            failed = sched.failed.len(),
            skipped = sched.skipped.len(),
            "wave runner: entering",
        );

        let mut wave_idx = 0u32;
        while !sched.remaining.is_empty() && !self.is_cancelled() {
            wave_idx += 1;
            tracing::debug!(
                iteration = iter.number,
                wave_idx,
                remaining = sched.remaining.len(),
                "wave runner: next loop",
            );
            let (ready, skip_now) = sched.next_wave();

            // Apply cascade-skips before we run anything.
            for id in &skip_now {
                sched.remaining.remove(id);
                sched.skipped.insert(id.clone());
                if let Some(row) = sched.task_rows.get(id) {
                    let _ = self.store.set_task_status(&row.id, TaskStatus::Skipped);
                }
            }
            if !skip_now.is_empty() {
                self.emit_for(
                    EventPayload::TasksSkipped {
                        count: skip_now.len(),
                        reason: "dependency_failed".into(),
                    },
                    Some(iter.id.clone()),
                    None,
                );
            }

            if ready.is_empty() {
                // Nothing scheduled but something is still remaining → cycle
                // or orphan dep. Mark everything as skipped and bail.
                if !sched.remaining.is_empty() {
                    for id in sched.remaining.iter() {
                        if let Some(row) = sched.task_rows.get(id) {
                            let _ = self.store.set_task_status(&row.id, TaskStatus::Skipped);
                        }
                    }
                    self.emit(EventPayload::GraphDeadlock);
                }
                break;
            }

            self.emit_for(
                EventPayload::WaveStarted { size: ready.len() },
                Some(iter.id.clone()),
                None,
            );

            // Run wave in parallel, serialize merges via spawned tasks +
            // sequential await on handles. Each `run_specialist_task` does
            // its own commit + merge into `main`.
            let wave_started = std::time::Instant::now();
            tracing::info!(
                iteration = iter.number,
                wave_idx,
                wave_size = ready.len(),
                "wave: spawning tasks",
            );
            let mut handles = Vec::new();
            for t in ready.iter().cloned() {
                let Some(row) = sched.task_rows.get(&t.id).cloned() else {
                    tracing::warn!(arch_id = %t.id, "wave: no task row, skipping");
                    sched.remaining.remove(&t.id);
                    sched.skipped.insert(t.id.clone());
                    continue;
                };
                let project = project.clone();
                let iter = iter.clone();
                let me = self.clone();
                let arch_id = t.id.clone();
                handles.push(tokio::spawn(async move {
                    let r = me.run_specialist_task(project, iter, t, row).await;
                    (arch_id, r)
                }));
            }
            let handle_count = handles.len();
            for (i, h) in handles.into_iter().enumerate() {
                tracing::debug!(iteration = iter.number, wave_idx, awaiting = i + 1, of = handle_count, "wave: awaiting task handle");
                let (arch_id, res) = h.await.unwrap_or((String::new(), Ok(false)));
                tracing::debug!(iteration = iter.number, wave_idx, arch_id = %arch_id, ok = matches!(res, Ok(true)), "wave: task handle resolved");
                sched.remaining.remove(&arch_id);
                match res {
                    Ok(true) => {
                        sched.completed.insert(arch_id);
                    }
                    _ => {
                        sched.failed.insert(arch_id);
                    }
                }
            }
            tracing::info!(
                iteration = iter.number,
                wave_idx,
                wave_ms = wave_started.elapsed().as_millis() as u64,
                remaining = sched.remaining.len(),
                completed = sched.completed.len(),
                failed = sched.failed.len(),
                "wave: completed",
            );
            // wrap-up doesn't interrupt the current iteration — it only
            // prevents the next one from starting. So no action here per wave;
            // the outer loop reads `wrap_up_requested` after the iteration ends.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str]) -> ArchTask {
        ArchTask {
            id: id.into(),
            role: AgentRole::SpecialistBackend,
            title: format!("task {id}"),
            description: String::new(),
            depends_on: deps.iter().map(|s| (*s).into()).collect(),
        }
    }

    /// All tasks pending, deps fully linear: first wave picks only the root.
    #[test]
    fn linear_dag_runs_root_first() {
        let tasks = vec![task("a", &[]), task("b", &["a"]), task("c", &["b"])];
        let s = Scheduler::from_store(&tasks, HashMap::new());
        let (ready, skip) = s.next_wave();
        assert_eq!(skip.len(), 0);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");
    }

    /// Two independent tasks should both be ready in the first wave.
    #[test]
    fn independent_tasks_run_in_parallel() {
        let tasks = vec![task("a", &[]), task("b", &[])];
        let s = Scheduler::from_store(&tasks, HashMap::new());
        let (ready, _) = s.next_wave();
        assert_eq!(ready.len(), 2);
    }

    /// Tasks whose dep already failed should cascade-skip.
    #[test]
    fn cascade_skip_on_failed_dep() {
        let tasks = vec![task("a", &[]), task("b", &["a"]), task("c", &["b"])];
        let mut s = Scheduler::from_store(&tasks, HashMap::new());
        s.remaining.remove("a");
        s.failed.insert("a".into());
        let (ready, skip) = s.next_wave();
        assert!(ready.is_empty(), "no task should run after dep failed");
        assert!(skip.contains(&"b".to_string()));
    }

    /// MAX_CONCURRENCY ceiling — verifies the cap kicks in when more tasks
    /// are ready than the limit. We create cap+5 tasks all without deps.
    #[test]
    fn next_wave_is_capped_at_max_concurrency() {
        let many: Vec<ArchTask> = (0..(MAX_CONCURRENCY + 5))
            .map(|i| task(&format!("t{i}"), &[]))
            .collect();
        let s = Scheduler::from_store(&many, HashMap::new());
        let (ready, _) = s.next_wave();
        assert_eq!(ready.len(), MAX_CONCURRENCY);
    }

    /// An unknown dep (not in by_id) is treated as already-satisfied, not as
    /// a blocker — useful for resume scenarios.
    #[test]
    fn unknown_dep_is_treated_as_satisfied() {
        let tasks = vec![task("a", &["ghost"])];
        let s = Scheduler::from_store(&tasks, HashMap::new());
        let (ready, _) = s.next_wave();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");
    }
}
