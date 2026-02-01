//! Runtime execution tasks (Story 2.3)
//!
//! This module provides the async task loops for the execution pipeline.
//! The execution task consumes SpreadOpportunity messages and triggers
//! delta-neutral trades.

use tokio::sync::{broadcast, mpsc};
use tracing::{error, info};

use crate::adapters::ExchangeAdapter;
use crate::core::channels::SpreadOpportunity;
use crate::core::execution::DeltaNeutralExecutor;

/// Execution task that processes spread opportunities
///
/// Listens for `SpreadOpportunity` messages on the channel and executes
/// delta-neutral trades. Shuts down gracefully on shutdown signal.
///
/// # Arguments
/// * `opportunity_rx` - Receiver for spread opportunities
/// * `executor` - The DeltaNeutralExecutor for trade execution
/// * `shutdown_rx` - Broadcast receiver for shutdown signal
pub async fn execution_task<V, P>(
    mut opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    executor: DeltaNeutralExecutor<V, P>,
    mut shutdown_rx: broadcast::Receiver<()>,
) where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    info!("Execution task started");

    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                info!("Execution task shutting down");
                break;
            }
            // Process incoming opportunities
            Some(opportunity) = opportunity_rx.recv() => {
                info!(
                    pair = %opportunity.pair,
                    spread = %format!("{:.4}%", opportunity.spread_percent),
                    direction = ?opportunity.direction,
                    "Processing spread opportunity"
                );

                match executor.execute_delta_neutral(opportunity).await {
                    Ok(result) => {
                        if result.success {
                            info!(
                                latency_ms = result.execution_latency_ms,
                                long = %result.long_exchange,
                                short = %result.short_exchange,
                                "Delta-neutral trade executed successfully"
                            );
                        } else {
                            error!(
                                latency_ms = result.execution_latency_ms,
                                long_success = %result.long_order.is_success(),
                                short_success = %result.short_order.is_success(),
                                "Delta-neutral trade partially failed"
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = ?e, "Delta-neutral execution error");
                    }
                }
            }
        }
    }

    info!("Execution task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ExchangeResult;
    use crate::adapters::types::{OrderRequest, OrderResponse, OrderStatus, Orderbook, PositionInfo};
    use crate::core::spread::SpreadDirection;
    use async_trait::async_trait;
    use tokio::time::{timeout, Duration};

    /// Simple mock adapter for runtime tests
    struct RuntimeMockAdapter {
        name: &'static str,
    }

    impl RuntimeMockAdapter {
        fn new(name: &'static str) -> Self {
            Self { name }
        }
    }

    #[async_trait]
    impl ExchangeAdapter for RuntimeMockAdapter {
        async fn connect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn disconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn subscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        async fn unsubscribe_orderbook(&mut self, _symbol: &str) -> ExchangeResult<()> { Ok(()) }
        
        async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse> {
            Ok(OrderResponse {
                order_id: format!("{}-{}", self.name, order.client_order_id),
                client_order_id: order.client_order_id,
                status: OrderStatus::Filled,
                filled_quantity: order.quantity,
                avg_price: Some(42000.0),
            })
        }
        
        async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> { Ok(()) }
        fn get_orderbook(&self, _symbol: &str) -> Option<&Orderbook> { None }
        fn is_connected(&self) -> bool { true }
        fn is_stale(&self) -> bool { false }
        async fn sync_orderbooks(&mut self) {}
        async fn reconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
        async fn get_position(&self, _symbol: &str) -> ExchangeResult<Option<PositionInfo>> { Ok(None) }
        fn exchange_name(&self) -> &'static str { self.name }
    }

    #[tokio::test]
    async fn test_execution_task_processes_opportunity() {
        let (opportunity_tx, opportunity_rx) = mpsc::channel(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        // Spawn the execution task
        let handle = tokio::spawn(async move {
            execution_task(opportunity_rx, executor, shutdown_rx).await;
        });

        // Send an opportunity
        let opportunity = SpreadOpportunity {
            pair: "BTC-PERP".to_string(),
            dex_a: "vest".to_string(),
            dex_b: "paradex".to_string(),
            spread_percent: 0.35,
            direction: SpreadDirection::AOverB,
            detected_at_ms: 1706000000000,
        };
        opportunity_tx.send(opportunity).await.unwrap();

        // Give time for processing
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Shutdown
        let _ = shutdown_tx.send(());

        // Wait for task to complete
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Task should complete on shutdown");
    }

    #[tokio::test]
    async fn test_execution_task_shutdown() {
        let (_opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(10);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let vest = RuntimeMockAdapter::new("vest");
        let paradex = RuntimeMockAdapter::new("paradex");
        let executor = DeltaNeutralExecutor::new(
            vest,
            paradex,
            0.01,
            "BTC-PERP".to_string(),
            "BTC-USD-PERP".to_string(),
        );

        let handle = tokio::spawn(async move {
            execution_task(opportunity_rx, executor, shutdown_rx).await;
        });

        // Send shutdown immediately
        let _ = shutdown_tx.send(());

        // Task should terminate quickly
        let result = timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Task should shutdown gracefully");
    }
}
