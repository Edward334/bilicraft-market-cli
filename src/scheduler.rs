use crate::{app::AppState, risk};
use chrono::Utc;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub fn spawn(state: AppState) {
    let interval = state.config.ai.automation_interval_seconds.max(30);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(interval));
        loop {
            timer.tick().await;
            if !state.ai_enabled.load(Ordering::Relaxed)
                || !state.automation_enabled.load(Ordering::Relaxed)
                || state.circuit_breaker.load(Ordering::Relaxed)
                || !risk::in_windows(Utc::now(), &state.config.risk.automation_windows)
            {
                continue;
            }
            state.emit(
                "ai_cycle",
                "AI automation cycle started",
                serde_json::json!({}),
            );
            let url = format!("http://{}/api/ai/run", state.config.web.bind);
            let response = state
                .client
                .post(url)
                .json(&serde_json::json!({}))
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => state.emit(
                    "ai_cycle",
                    "AI automation cycle completed",
                    serde_json::json!({"status":response.status().as_u16()}),
                ),
                Ok(response) => {
                    let message =
                        format!("AI automation cycle rejected: HTTP {}", response.status());
                    state
                        .db
                        .lock()
                        .audit("ai_cycle", "scheduler", "failed", &message)
                        .ok();
                    state.emit("error", &message, serde_json::json!({}));
                }
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
