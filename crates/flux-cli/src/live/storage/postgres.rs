//! PostgreSQL storage backend implementation.
//!
//! Provides `PostgresBackend` which persists all harness data (fills, signals,
//! risk events, positions, equity snapshots, orders, checkpoints) to a
//! PostgreSQL database using `sqlx` with connection pooling.
//!
//! Each trading account gets its own schema, providing data isolation.

use async_trait::async_trait;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::time::Duration;

use super::{
    EquitySnapshot, FillInfo, FillRecord, OrderRecord, PositionRecord, RiskEventRecord,
    SignalRecord, StorageBackend, StorageResult,
};
use crate::live::state::HarnessState;

/// Error type for PostgreSQL storage operations.
#[derive(Debug, thiserror::Error)]
pub enum PgStorageError {
    /// A database operation failed.
    #[error("{operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
    /// An order with the given ID was not found.
    #[error("order not found: {order_id}")]
    OrderNotFound { order_id: String },
    /// The provided schema name is invalid.
    #[error("invalid schema name: {name}")]
    InvalidSchema { name: String },
}

/// Validate a PostgreSQL schema name.
///
/// Accepts only non-empty strings composed of lowercase alphanumeric
/// characters and underscores (`[a-z0-9_]+`).
pub fn validate_schema_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// PostgreSQL-backed storage for the live trading harness.
///
/// Maintains a connection pool and operates within a dedicated schema
/// for table isolation per trading account.
pub struct PostgresBackend {
    pool: PgPool,
    schema: String,
}

impl PostgresBackend {
    /// Connect to PostgreSQL and initialize the schema and tables.
    ///
    /// - Validates the schema name (must match `[a-z0-9_]+`)
    /// - Creates a connection pool (max 5 connections, 5s acquire timeout)
    /// - Creates the schema if it doesn't exist
    /// - Creates all 7 tables if they don't exist
    pub async fn new(database_url: &str, schema: &str) -> StorageResult<Self> {
        if !validate_schema_name(schema) {
            return Err(Box::new(PgStorageError::InvalidSchema {
                name: schema.to_string(),
            }));
        }

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "connect",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        let backend = Self {
            pool,
            schema: schema.to_string(),
        };

        backend.initialize_schema().await?;

        Ok(backend)
    }

    /// Run CREATE SCHEMA + all CREATE TABLE IF NOT EXISTS statements.
    async fn initialize_schema(&self) -> StorageResult<()> {
        let sql = format!(
            r#"
CREATE SCHEMA IF NOT EXISTS {schema};

CREATE TABLE IF NOT EXISTS {schema}.fills (
    id          BIGSERIAL PRIMARY KEY,
    ts          TIMESTAMPTZ NOT NULL,
    strategy    TEXT NOT NULL,
    symbol      TEXT NOT NULL,
    side        TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    qty         DOUBLE PRECISION NOT NULL CHECK (qty > 0),
    price       DOUBLE PRECISION NOT NULL CHECK (price > 0),
    order_id    TEXT,
    latency_ms  INTEGER,
    bar_index   BIGINT NOT NULL CHECK (bar_index >= 0)
);

CREATE TABLE IF NOT EXISTS {schema}.signals (
    id          BIGSERIAL PRIMARY KEY,
    ts          TIMESTAMPTZ NOT NULL,
    strategy    TEXT NOT NULL,
    symbol      TEXT NOT NULL,
    signal_type TEXT NOT NULL CHECK (signal_type IN ('open', 'short', 'close', 'close_qty')),
    qty         DOUBLE PRECISION CHECK (qty > 0),
    decision    TEXT NOT NULL CHECK (decision IN ('allow', 'reject', 'flatten_all')),
    reject_reason TEXT
);

CREATE TABLE IF NOT EXISTS {schema}.risk_events (
    id          BIGSERIAL PRIMARY KEY,
    ts          TIMESTAMPTZ NOT NULL,
    event_type  TEXT NOT NULL,
    details     JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS {schema}.positions (
    symbol      TEXT PRIMARY KEY,
    qty         DOUBLE PRECISION NOT NULL DEFAULT 0,
    avg_entry   DOUBLE PRECISION NOT NULL DEFAULT 0,
    realized_pnl DOUBLE PRECISION NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS {schema}.equity_snapshots (
    ts          TIMESTAMPTZ PRIMARY KEY,
    equity      DOUBLE PRECISION NOT NULL,
    equity_peak DOUBLE PRECISION NOT NULL,
    daily_pnl   DOUBLE PRECISION NOT NULL,
    weekly_pnl  DOUBLE PRECISION NOT NULL,
    drawdown_pct DOUBLE PRECISION NOT NULL CHECK (drawdown_pct >= 0),
    open_positions INTEGER NOT NULL CHECK (open_positions >= 0)
);

CREATE TABLE IF NOT EXISTS {schema}.orders (
    id          TEXT PRIMARY KEY,
    ts          TIMESTAMPTZ NOT NULL,
    symbol      TEXT NOT NULL,
    side        TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    qty         DOUBLE PRECISION NOT NULL CHECK (qty > 0),
    order_type  TEXT NOT NULL CHECK (order_type IN ('market', 'limit')),
    status      TEXT NOT NULL CHECK (status IN ('submitted', 'acknowledged', 'filled', 'partial', 'rejected', 'cancelled')),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    fill_price  DOUBLE PRECISION,
    fill_qty    DOUBLE PRECISION
);

CREATE TABLE IF NOT EXISTS {schema}.checkpoints (
    id          BIGSERIAL PRIMARY KEY,
    ts          TIMESTAMPTZ NOT NULL DEFAULT now(),
    state       JSONB NOT NULL,
    bars_processed BIGINT NOT NULL
);
"#,
            schema = self.schema
        );

        sqlx::query(&sql).execute(&self.pool).await.map_err(|e| {
            Box::new(PgStorageError::Database {
                operation: "initialize_schema",
                source: e,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(())
    }
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    async fn record_fill(&self, fill: &FillRecord) -> StorageResult<()> {
        let sql = format!(
            "INSERT INTO {}.fills (ts, strategy, symbol, side, qty, price, order_id, latency_ms, bar_index) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            self.schema
        );

        sqlx::query(&sql)
            .bind(fill.timestamp)
            .bind(&fill.strategy)
            .bind(&fill.symbol)
            .bind(&fill.side)
            .bind(fill.qty)
            .bind(fill.price)
            .bind(&fill.order_id)
            .bind(fill.latency_ms)
            .bind(fill.bar_index)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "record_fill",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn record_signal(&self, signal: &SignalRecord) -> StorageResult<()> {
        let sql = format!(
            "INSERT INTO {}.signals (ts, strategy, symbol, signal_type, qty, decision, reject_reason) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            self.schema
        );

        sqlx::query(&sql)
            .bind(signal.timestamp)
            .bind(&signal.strategy)
            .bind(&signal.symbol)
            .bind(&signal.signal_type)
            .bind(signal.qty)
            .bind(&signal.decision)
            .bind(&signal.reject_reason)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "record_signal",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn record_risk_event(&self, event: &RiskEventRecord) -> StorageResult<()> {
        let sql = format!(
            "INSERT INTO {}.risk_events (ts, event_type, details) VALUES ($1, $2, $3)",
            self.schema
        );

        sqlx::query(&sql)
            .bind(event.timestamp)
            .bind(&event.event_type)
            .bind(&event.details)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "record_risk_event",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn upsert_position(&self, symbol: &str, qty: f64, avg_entry: f64) -> StorageResult<()> {
        let sql = format!(
            "INSERT INTO {}.positions (symbol, qty, avg_entry, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (symbol) DO UPDATE SET qty = $2, avg_entry = $3, updated_at = now()",
            self.schema
        );

        sqlx::query(&sql)
            .bind(symbol)
            .bind(qty)
            .bind(avg_entry)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "upsert_position",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn snapshot_equity(&self, snapshot: &EquitySnapshot) -> StorageResult<()> {
        let sql = format!(
            "INSERT INTO {}.equity_snapshots (ts, equity, equity_peak, daily_pnl, weekly_pnl, drawdown_pct, open_positions) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            self.schema
        );

        sqlx::query(&sql)
            .bind(snapshot.timestamp)
            .bind(snapshot.equity)
            .bind(snapshot.equity_peak)
            .bind(snapshot.daily_pnl)
            .bind(snapshot.weekly_pnl)
            .bind(snapshot.drawdown_pct)
            .bind(snapshot.open_positions)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "snapshot_equity",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn save_checkpoint(&self, state: &HarnessState) -> StorageResult<()> {
        let state_json = serde_json::to_value(state).map_err(|e| {
            Box::new(PgStorageError::Database {
                operation: "save_checkpoint",
                source: sqlx::Error::Protocol(e.to_string()),
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        let sql = format!(
            "INSERT INTO {}.checkpoints (state, bars_processed) VALUES ($1, $2)",
            self.schema
        );

        sqlx::query(&sql)
            .bind(&state_json)
            .bind(state.bars_processed as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "save_checkpoint",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn load_latest_checkpoint(&self) -> StorageResult<Option<HarnessState>> {
        let sql = format!(
            "SELECT state FROM {}.checkpoints ORDER BY ts DESC LIMIT 1",
            self.schema
        );

        let row = sqlx::query(&sql)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "load_latest_checkpoint",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        match row {
            Some(row) => {
                let state_json: serde_json::Value = row.try_get("state").map_err(|e| {
                    Box::new(PgStorageError::Database {
                        operation: "load_latest_checkpoint",
                        source: e,
                    }) as Box<dyn std::error::Error + Send + Sync>
                })?;
                let state: HarnessState =
                    serde_json::from_value(state_json).map_err(|e| {
                        Box::new(PgStorageError::Database {
                            operation: "load_latest_checkpoint",
                            source: sqlx::Error::Protocol(e.to_string()),
                        }) as Box<dyn std::error::Error + Send + Sync>
                    })?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    async fn load_positions(&self) -> StorageResult<Vec<PositionRecord>> {
        let sql = format!(
            "SELECT symbol, qty, avg_entry, realized_pnl, updated_at FROM {}.positions",
            self.schema
        );

        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "load_positions",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        let positions = rows
            .iter()
            .map(|row| {
                Ok(PositionRecord {
                    symbol: row.try_get("symbol")?,
                    qty: row.try_get("qty")?,
                    avg_entry: row.try_get("avg_entry")?,
                    realized_pnl: row.try_get("realized_pnl")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "load_positions",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(positions)
    }

    async fn record_order(&self, order: &OrderRecord) -> StorageResult<()> {
        let sql = format!(
            "INSERT INTO {}.orders (id, ts, symbol, side, qty, order_type, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            self.schema
        );

        sqlx::query(&sql)
            .bind(&order.id)
            .bind(order.timestamp)
            .bind(&order.symbol)
            .bind(&order.side)
            .bind(order.qty)
            .bind(&order.order_type)
            .bind(&order.status)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Box::new(PgStorageError::Database {
                    operation: "record_order",
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;

        Ok(())
    }

    async fn update_order_status(
        &self,
        order_id: &str,
        status: &str,
        fill_info: Option<&FillInfo>,
    ) -> StorageResult<()> {
        let result = match fill_info {
            Some(info) => {
                let sql = format!(
                    "UPDATE {}.orders SET status = $1, updated_at = now(), fill_price = $2, fill_qty = $3 \
                     WHERE id = $4",
                    self.schema
                );
                sqlx::query(&sql)
                    .bind(status)
                    .bind(info.fill_price)
                    .bind(info.fill_qty)
                    .bind(order_id)
                    .execute(&self.pool)
                    .await
            }
            None => {
                let sql = format!(
                    "UPDATE {}.orders SET status = $1, updated_at = now() WHERE id = $2",
                    self.schema
                );
                sqlx::query(&sql)
                    .bind(status)
                    .bind(order_id)
                    .execute(&self.pool)
                    .await
            }
        };

        let result = result.map_err(|e| {
            Box::new(PgStorageError::Database {
                operation: "update_order_status",
                source: e,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        if result.rows_affected() == 0 {
            return Err(Box::new(PgStorageError::OrderNotFound {
                order_id: order_id.to_string(),
            }));
        }

        Ok(())
    }
}
