use crate::{app::AppState, risk};
use chrono::Utc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        let mut last_cycle: Option<Instant> = None;
        let mut was_enabled = false;
        loop {
            timer.tick().await;
            let config = state.config_snapshot();
            let enabled = state.ai_enabled.load(Ordering::Relaxed)
                && state.automation_enabled.load(Ordering::Relaxed);
            if !enabled {
                was_enabled = false;
                continue;
            }
            if !was_enabled {
                last_cycle = None;
                was_enabled = true;
            }
            if config.risk.optional_limits_enabled
                && (state.circuit_breaker.load(Ordering::Relaxed)
                    || !risk::in_windows(Utc::now(), &config.risk.automation_windows))
            {
                continue;
            }
            let interval = Duration::from_secs(config.ai.automation_interval_seconds.max(30));
            if last_cycle.is_some_and(|last| last.elapsed() < interval) {
                continue;
            }
            last_cycle = Some(Instant::now());
            state.emit(
                "ai_cycle",
                "AI automation cycle started",
                serde_json::json!({}),
            );
            // Call the shared executor directly; internal jobs do not use Web login cookies.
            match crate::server::run_automation(state.clone()).await {
                Ok(()) => state.emit(
                    "ai_cycle",
                    "AI automation cycle completed",
                    serde_json::json!({}),
                ),
                Err(error) => {
                    let message = format!("AI automation cycle failed: {error}");
                    state
                        .db
                        .lock()
                        .audit("ai_cycle", "scheduler", "failed", &message)
                        .ok();
                    state.emit("error", &message, serde_json::json!({}));
                }
            }
        }
    });
}
