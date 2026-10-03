use crate::{
    db::Database,
    domain::{PortfolioSnapshot, StockHolding},
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PurchaseRecord {
    pub company_id: i64,
    pub company_name: String,
    pub shares: i64,
    pub total_cost: f64,
}
impl PurchaseRecord {
    pub fn average_price(&self) -> f64 {
        if self.shares > 0 {
            self.total_cost / self.shares as f64
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfitRecord {
    pub day: String,
    pub timestamp: DateTime<Utc>,
    pub total_pnl: f64,
    pub total_pnl_pct: Option<f64>,
    pub invested_capital: f64,
    pub total_assets: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AiCapital {
    pub principal: f64,
    pub realized_pnl: f64,
    pub invested_cost: f64,
    pub total_capital: f64,
    pub available_cash: f64,
    pub cash_verified: bool,
}

impl Database {
    pub fn init_ledger(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS purchase_records (company_id INTEGER PRIMARY KEY, company_name TEXT NOT NULL, shares INTEGER NOT NULL, total_cost REAL NOT NULL); CREATE TABLE IF NOT EXISTS profit_history (day TEXT PRIMARY KEY, timestamp TEXT NOT NULL, total_pnl REAL NOT NULL, total_pnl_pct REAL, invested_capital REAL NOT NULL, total_assets REAL); CREATE TABLE IF NOT EXISTS profit_meta (key TEXT PRIMARY KEY, value REAL NOT NULL);")?;
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS ai_positions (company_id INTEGER PRIMARY KEY, company_name TEXT NOT NULL, shares INTEGER NOT NULL, total_cost REAL NOT NULL); CREATE TABLE IF NOT EXISTS ai_trades (id INTEGER PRIMARY KEY, company_id INTEGER NOT NULL, shares INTEGER NOT NULL, price REAL NOT NULL, realized_pnl REAL NOT NULL, executed_at TEXT NOT NULL);")?;
        Ok(())
    }
    pub fn ai_positions(&self) -> Result<Vec<PurchaseRecord>> {
        let mut q = self.conn.prepare("SELECT company_id,company_name,shares,total_cost FROM ai_positions WHERE shares>0 ORDER BY company_id")?;
        Ok(q.query_map([], |r| {
            Ok(PurchaseRecord {
                company_id: r.get(0)?,
                company_name: r.get(1)?,
                shares: r.get(2)?,
                total_cost: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn ai_capital(&self, principal: f64, cash: Option<f64>) -> Result<AiCapital> {
        let invested_cost = self
            .ai_positions()?
            .iter()
            .map(|p| p.total_cost)
            .sum::<f64>()
            .max(0.0);
        let realized_pnl: f64 = self.conn.query_row(
            "SELECT COALESCE(SUM(realized_pnl),0) FROM ai_trades",
            [],
            |r| r.get(0),
        )?;
        let total_capital = principal + realized_pnl;
        let remaining = (total_capital - invested_cost).max(0.0);
        Ok(AiCapital {
            principal,
            realized_pnl,
            invested_cost,
            total_capital,
            available_cash: cash.map(|c| remaining.min(c.max(0.0))).unwrap_or(remaining),
            cash_verified: cash.is_some(),
        })
    }
    pub fn record_owned_buy(
        &self,
        ai: bool,
        id: i64,
        name: &str,
        shares: i64,
        price: f64,
    ) -> Result<()> {
        anyhow::ensure!(
            shares > 0 && price.is_finite() && price > 0.0,
            "invalid confirmed trade"
        );
        let tx = self.conn.unchecked_transaction()?;
        self.record_buy(id, name, shares, price)?;
        if ai {
            self.conn.execute("INSERT INTO ai_positions(company_id,company_name,shares,total_cost) VALUES(?1,?2,?3,?4) ON CONFLICT(company_id) DO UPDATE SET shares=shares+excluded.shares,total_cost=total_cost+excluded.total_cost", params![id,name,shares,price*shares as f64])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn record_owned_sell(&self, ai: bool, id: i64, shares: i64, price: f64) -> Result<()> {
        anyhow::ensure!(
            shares > 0 && price.is_finite() && price > 0.0,
            "invalid confirmed trade"
        );
        let tx = self.conn.unchecked_transaction()?;
        let position = self
            .ai_positions()?
            .into_iter()
            .find(|p| p.company_id == id);
        if let Some(p) = position {
            let all = self
                .purchase_records()?
                .into_iter()
                .find(|p| p.company_id == id)
                .map(|p| p.shares)
                .unwrap_or(0);
            let sold = if ai {
                shares.min(p.shares)
            } else {
                (shares - (all - p.shares).max(0)).max(0).min(p.shares)
            };
            if sold > 0 {
                let cost = p.average_price();
                self.conn.execute("UPDATE ai_positions SET shares=shares-?2,total_cost=MAX(0,total_cost-?3) WHERE company_id=?1", params![id,sold,cost*sold as f64])?;
                self.conn.execute("INSERT INTO ai_trades(company_id,shares,price,realized_pnl,executed_at) VALUES(?1,?2,?3,?4,?5)", params![id,sold,price,(price-cost)*sold as f64,Utc::now().to_rfc3339()])?;
            }
        }
        self.record_sell(id, shares, price, false)?;
        tx.commit()?;
        Ok(())
    }
    pub fn purchase_records(&self) -> Result<Vec<PurchaseRecord>> {
        let mut s=self.conn.prepare("SELECT company_id,company_name,shares,total_cost FROM purchase_records WHERE shares>0 ORDER BY company_id")?;
        Ok(s.query_map([], |r| {
            Ok(PurchaseRecord {
                company_id: r.get(0)?,
                company_name: r.get(1)?,
                shares: r.get(2)?,
                total_cost: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn record_buy(&self, company_id: i64, name: &str, shares: i64, price: f64) -> Result<()> {
        if shares <= 0 || !price.is_finite() || price < 0.0 {
            return Ok(());
        }
        self.conn.execute("INSERT INTO purchase_records(company_id,company_name,shares,total_cost) VALUES(?1,?2,?3,?4) ON CONFLICT(company_id) DO UPDATE SET company_name=excluded.company_name,shares=shares+excluded.shares,total_cost=total_cost+excluded.total_cost",params![company_id,name,shares,price*shares as f64])?;
        Ok(())
    }
    pub fn record_sell(
        &self,
        company_id: i64,
        shares: i64,
        sell_price: f64,
        fully_sold: bool,
    ) -> Result<()> {
        if shares <= 0 || !sell_price.is_finite() || sell_price < 0.0 {
            return Ok(());
        }
        let current = self
            .conn
            .query_row(
                "SELECT shares,total_cost FROM purchase_records WHERE company_id=?1",
                params![company_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
            )
            .optional()?;
        if let Some((held, cost)) = current {
            if held <= 0 {
                return Ok(());
            }
            let sold = if fully_sold { held } else { shares.min(held) };
            let average_cost = cost / held as f64;
            let realized = (sell_price - average_cost) * sold as f64;
            let previous = self.realized_pnl()?;
            self.set_realized_pnl(previous + realized)?;
            let remain = if fully_sold {
                0
            } else {
                (held - shares).max(0)
            };
            if remain == 0 {
                self.conn.execute(
                    "DELETE FROM purchase_records WHERE company_id=?1",
                    params![company_id],
                )?;
            } else {
                self.conn.execute(
                    "UPDATE purchase_records SET shares=?2,total_cost=?3 WHERE company_id=?1",
                    params![company_id, remain, cost / held as f64 * remain as f64],
                )?;
            }
        }
        Ok(())
    }
    pub fn set_realized_pnl(&self, value: f64) -> Result<()> {
        self.set_meta("realized_pnl", value)
    }
    pub fn realized_pnl(&self) -> Result<f64> {
        Ok(self.get_meta("realized_pnl")?.unwrap_or(0.0))
    }
    fn set_meta(&self, key: &str, value: f64) -> Result<()> {
        self.conn.execute("INSERT INTO profit_meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        Ok(())
    }
    fn get_meta(&self, key: &str) -> Result<Option<f64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM profit_meta WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn record_profit_snapshot(&self, snapshot: &PortfolioSnapshot) -> Result<ProfitRecord> {
        self.init_ledger()?;
        let invested = snapshot.tracked_cost.max(0.0);
        let pnl = snapshot.realized_pnl + snapshot.unrealized_pnl;
        let pct = if snapshot.total_assets > 0.0 {
            Some(pnl / snapshot.total_assets * 100.0)
        } else {
            None
        };
        let now = Utc::now();
        let day = now.format("%Y-%m-%d").to_string();
        let record = ProfitRecord {
            day: day.clone(),
            timestamp: now,
            total_pnl: pnl,
            total_pnl_pct: pct,
            invested_capital: invested,
            total_assets: Some(snapshot.total_assets),
        };
        self.conn.execute("INSERT INTO profit_history(day,timestamp,total_pnl,total_pnl_pct,invested_capital,total_assets) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(day) DO UPDATE SET timestamp=excluded.timestamp,total_pnl=excluded.total_pnl,total_pnl_pct=excluded.total_pnl_pct,invested_capital=excluded.invested_capital,total_assets=excluded.total_assets",params![record.day,record.timestamp.to_rfc3339(),record.total_pnl,record.total_pnl_pct,record.invested_capital,record.total_assets])?;
        Ok(record)
    }
    pub fn profit_history(&self, limit: u32) -> Result<Vec<ProfitRecord>> {
        self.init_ledger()?;
        let mut s=self.conn.prepare("SELECT day,timestamp,total_pnl,total_pnl_pct,invested_capital,total_assets FROM profit_history ORDER BY timestamp DESC LIMIT ?1")?;
        Ok(s.query_map(params![limit], |r| {
            Ok(ProfitRecord {
                day: r.get(0)?,
                timestamp: r
                    .get::<_, String>(1)?
                    .parse()
                    .unwrap_or_else(|_| Utc::now()),
                total_pnl: r.get(2)?,
                total_pnl_pct: r.get(3)?,
                invested_capital: r.get(4)?,
                total_assets: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn holding_metrics(&self, holdings: &[StockHolding]) -> Result<Vec<serde_json::Value>> {
        let records = self.purchase_records()?;
        Ok(holdings.iter().map(|h|{let r=records.iter().find(|x|x.company_id==h.market_id.unwrap_or(-1));let cost=r.map(|x|x.total_cost).unwrap_or(0.0);let pnl=h.total_value-cost;serde_json::json!({"company_name":h.company_name,"shares":h.shares,"total_value":h.total_value,"market_id":h.market_id,"cost":cost,"pnl":pnl,"pnl_pct":if cost>0.0{Some(pnl/cost*100.0)}else{None::<f64>}})}).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ai_capital_reinvests_profits_and_excludes_manual_trades() {
        let root = tempdir().unwrap();
        let db = Database::open(&root.path().join("capital.sqlite3")).unwrap();
        db.init_ledger().unwrap();
        db.record_owned_buy(false, 0, "Manual", 10, 100.0).unwrap();
        db.record_owned_sell(false, 0, 10, 200.0).unwrap();
        assert_eq!(db.ai_capital(200000.0, None).unwrap().realized_pnl, 0.0);
        db.record_owned_buy(true, 7, "AI", 500, 100.0).unwrap();
        assert_eq!(
            db.ai_capital(200000.0, Some(300000.0))
                .unwrap()
                .available_cash,
            150000.0
        );
        db.record_owned_sell(true, 7, 500, 110.0).unwrap();
        let capital = db.ai_capital(200000.0, Some(300000.0)).unwrap();
        assert_eq!(capital.total_capital, 205000.0);
        assert_eq!(capital.available_cash, 205000.0);
        db.record_owned_buy(true, 7, "AI", 100, 100.0).unwrap();
        db.record_owned_sell(true, 7, 100, 80.0).unwrap();
        assert_eq!(
            db.ai_capital(200000.0, None).unwrap().total_capital,
            203000.0
        );
        assert_eq!(
            db.ai_capital(200000.0, Some(50.0)).unwrap().available_cash,
            50.0
        );
        drop(db);
        let restarted = Database::open(&root.path().join("capital.sqlite3")).unwrap();
        restarted.init_ledger().unwrap();
        assert_eq!(
            restarted.ai_capital(200000.0, None).unwrap().total_capital,
            203000.0
        );
    }

    #[test]
    fn manual_liquidation_debits_ai_only_after_manual_shares() {
        let root = tempdir().unwrap();
        let db = Database::open(&root.path().join("mixed.sqlite3")).unwrap();
        db.init_ledger().unwrap();
        db.record_owned_buy(false, 0, "Shared", 5, 100.0).unwrap();
        db.record_owned_buy(true, 0, "Shared", 5, 100.0).unwrap();
        db.record_owned_sell(false, 0, 6, 110.0).unwrap();
        let capital = db.ai_capital(1000.0, None).unwrap();
        assert_eq!(capital.realized_pnl, 10.0);
        assert_eq!(capital.invested_cost, 400.0);
        assert_eq!(db.ai_positions().unwrap()[0].shares, 4);
    }

    #[test]
    fn records_realized_pnl_at_sell_price() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("ledger.sqlite3")).unwrap();
        database.init_ledger().unwrap();
        database.record_buy(0, "测试股票", 2, 100.0).unwrap();
        database.record_sell(0, 1, 110.0, false).unwrap();

        assert!((database.realized_pnl().unwrap() - 10.0).abs() < f64::EPSILON);
        let record = database.purchase_records().unwrap().pop().unwrap();
        assert_eq!(record.shares, 1);
        assert!((record.total_cost - 100.0).abs() < f64::EPSILON);
    }
}

use rusqlite::OptionalExtension;
