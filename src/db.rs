use crate::domain::{AuditEntry, PortfolioSnapshot};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::Path;

pub struct Database {
    pub conn: Connection,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }
    fn init(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS audit (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, source TEXT NOT NULL, status TEXT NOT NULL, message TEXT NOT NULL, created_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS watchlist (company_id INTEGER PRIMARY KEY); CREATE TABLE IF NOT EXISTS portfolio_snapshots (id INTEGER PRIMARY KEY, balance REAL NOT NULL, total_assets REAL NOT NULL, tracked_cost REAL NOT NULL, unrealized_pnl REAL NOT NULL, realized_pnl REAL NOT NULL, holdings_json TEXT NOT NULL, captured_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE INDEX IF NOT EXISTS audit_created_at ON audit(created_at);")?;
        Ok(())
    }
    pub fn audit(&self, kind: &str, source: &str, status: &str, message: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit(kind,source,status,message,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![kind, source, status, message, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
    pub fn audits(&self, limit: u32) -> Result<Vec<AuditEntry>> {
        let mut s = self.conn.prepare(
            "SELECT id,kind,source,status,message,created_at FROM audit ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = s.query_map(params![limit], |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                kind: r.get(1)?,
                source: r.get(2)?,
                status: r.get(3)?,
                message: r.get(4)?,
                created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(5)?)
                    .map(|v| v.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn save_snapshot(&self, snapshot: &PortfolioSnapshot) -> Result<()> {
        self.conn.execute("INSERT INTO portfolio_snapshots(balance,total_assets,tracked_cost,unrealized_pnl,realized_pnl,holdings_json,captured_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![snapshot.balance,snapshot.total_assets,snapshot.tracked_cost,snapshot.unrealized_pnl,snapshot.realized_pnl,serde_json::to_string(&snapshot.holdings)?,snapshot.captured_at.to_rfc3339()])?;
        Ok(())
    }
    pub fn daily_trade_count(&self) -> Result<u32> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        Ok(self.conn.query_row("SELECT COUNT(*) FROM audit WHERE kind='trade' AND status='success' AND substr(created_at,1,10)=?1", params![day], |r| r.get::<_, i64>(0))? as u32)
    }
    pub fn daily_loss(&self) -> Result<f64> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let current: Option<f64> = self
            .conn
            .query_row(
                "SELECT total_pnl FROM profit_history WHERE day=?1",
                params![today],
                |r| r.get(0),
            )
            .optional()?;
        let previous: Option<f64> = self
            .conn
            .query_row(
                "SELECT total_pnl FROM profit_history WHERE day<?1 ORDER BY day DESC LIMIT 1",
                params![today],
                |r| r.get(0),
            )
            .optional()?;
        Ok((current.unwrap_or(0.0) - previous.unwrap_or(0.0))
            .min(0.0)
            .abs())
    }
    pub fn last_successful_trade_at(&self) -> Result<Option<chrono::DateTime<Utc>>> {
        let value = self.conn.query_row("SELECT created_at FROM audit WHERE kind='trade' AND status='success' ORDER BY id DESC LIMIT 1", [], |r| r.get::<_, String>(0)).optional()?;
        Ok(value.and_then(|v| {
            chrono::DateTime::parse_from_rfc3339(&v)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }))
    }
    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        Ok(())
    }
    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM kv WHERE key=?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn watchlist(&self) -> Result<Vec<i64>> {
        let mut s = self
            .conn
            .prepare("SELECT company_id FROM watchlist ORDER BY company_id")?;
        Ok(s.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn set_watch(&self, id: i64, enabled: bool) -> Result<()> {
        if enabled {
            self.conn.execute(
                "INSERT OR IGNORE INTO watchlist(company_id) VALUES(?1)",
                params![id],
            )?;
        } else {
            self.conn
                .execute("DELETE FROM watchlist WHERE company_id=?1", params![id])?;
        }
        Ok(())
    }
}
use rusqlite::OptionalExtension;
