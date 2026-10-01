use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowState {
    MicrosoftSession,
    ConnectingMinecraft,
    ChatReady,
    MarketSyncing,
    AiDeciding,
    RiskChecking,
    Trading,
    Confirming,
    LedgerUpdating,
    Notifying,
    Paused,
    Failed,
}
pub struct Workflow {
    tx: watch::Sender<WorkflowState>,
}
impl Workflow {
    pub fn new() -> (Self, watch::Receiver<WorkflowState>) {
        let (tx, rx) = watch::channel(WorkflowState::MicrosoftSession);
        (Self { tx }, rx)
    }
    pub fn set(&self, s: WorkflowState) {
        let _ = self.tx.send(s);
        tracing::info!(state=?s,at=%Utc::now(),"workflow state changed")
    }
    pub fn current(&self) -> WorkflowState {
        *self.tx.borrow()
    }
}
