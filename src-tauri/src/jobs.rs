use parking_lot::Mutex;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Clone)]
pub struct JobControl {
    pub id: String,
    pub cancelled: Arc<AtomicBool>,
    pub progress_high_water: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct JobState {
    active: Mutex<Option<JobControl>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResult {
    pub output_path: String,
    pub model: String,
    pub provider: String,
    pub duration_ms: u64,
    pub frame_count: Option<u64>,
    pub preview: bool,
}

#[derive(Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobEvent {
    Progress {
        job_id: String,
        phase: String,
        completed: Option<u64>,
        total: Option<u64>,
        percent: Option<f64>,
        eta_seconds: Option<u64>,
        message: String,
    },
    Completed {
        job_id: String,
        result: ProcessResult,
    },
    Failed {
        job_id: String,
        error: String,
    },
    Cancelled {
        job_id: String,
    },
}

impl JobState {
    pub fn begin(&self) -> Result<JobControl, String> {
        let mut active = self.active.lock();
        if active.is_some() {
            return Err("Another download or processing job is already running".into());
        }
        let control = JobControl {
            id: Uuid::new_v4().to_string(),
            cancelled: Arc::new(AtomicBool::new(false)),
            progress_high_water: Arc::new(AtomicU64::new(0)),
        };
        *active = Some(control.clone());
        Ok(control)
    }

    pub fn cancel(&self, job_id: &str) -> Result<(), String> {
        let active = self.active.lock();
        let control = active.as_ref().ok_or("There is no active job")?;
        if control.id != job_id {
            return Err("The requested job is no longer active".into());
        }
        control.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn finish(&self, job_id: &str) {
        let mut active = self.active.lock();
        if active.as_ref().map(|job| job.id.as_str()) == Some(job_id) {
            *active = None;
        }
    }
}

pub fn emit(app: &AppHandle, event: JobEvent) {
    let _ = app.emit("job-event", event);
}

pub fn emit_progress(
    app: &AppHandle,
    control: &JobControl,
    phase: &str,
    completed: Option<u64>,
    total: Option<u64>,
    eta_seconds: Option<u64>,
    message: impl Into<String>,
) {
    let completed = completed.map(|value| {
        let previous = control
            .progress_high_water
            .fetch_max(value, Ordering::SeqCst);
        value.max(previous)
    });
    let percent = match (completed, total) {
        (Some(done), Some(all)) if all > 0 => {
            Some((done as f64 / all as f64 * 100.0).clamp(0.0, 100.0))
        }
        _ => None,
    };
    emit(
        app,
        JobEvent::Progress {
            job_id: control.id.clone(),
            phase: phase.into(),
            completed,
            total,
            percent,
            eta_seconds,
            message: message.into(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_foreground_job_can_run() {
        let state = JobState::default();
        let first = state.begin().expect("first job starts");
        assert!(state.begin().is_err());
        state.cancel(&first.id).expect("active job cancels");
        assert!(first.cancelled.load(Ordering::SeqCst));
        state.finish(&first.id);
        assert!(state.begin().is_ok());
    }
}
