use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn, error};
use crate::adapters::traits::ExchangeAdapter;

/// Configuration pour la reconnexion automatique
/// 
/// Note: La détection de connexion "stale" est gérée par la méthode
/// `ExchangeAdapter::is_stale()` de chaque adapter, pas par cette config.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Intervalle de vérification heartbeat (secondes)
    pub heartbeat_check_interval_secs: u64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            heartbeat_check_interval_secs: 30,  // Check toutes les 30s
        }
    }
}

/// Task de monitoring + reconnexion automatique
/// 
/// Cette task:
/// 1. Vérifie périodiquement si les adapters sont "stale" via is_stale()
/// 2. Si stale détecté, appelle adapter.reconnect()
/// 3. Se termine proprement sur shutdown signal
pub async fn reconnect_monitor_task<A>(
    adapter: Arc<Mutex<A>>,
    config: ReconnectConfig,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> anyhow::Result<()>
where
    A: ExchangeAdapter + Send + 'static,
{
    let check_interval = Duration::from_secs(config.heartbeat_check_interval_secs);
    let exchange_name = {
        let adapter_lock = adapter.lock().await;
        adapter_lock.exchange_name()
    };
    
    info!(exchange = exchange_name, "Reconnect monitor started");
    
    loop {
        tokio::select! {
            _ = tokio::time::sleep(check_interval) => {
                // Check if adapter is stale
                let is_stale = {
                    let adapter_lock = adapter.lock().await;
                    adapter_lock.is_stale()
                };
                
                if is_stale {
                    warn!(
                        exchange = exchange_name,
                        "[RECONNECT] Attempting reconnection to {}...", exchange_name
                    );
                    
                    let reconnect_result = {
                        let mut adapter_lock = adapter.lock().await;
                        adapter_lock.reconnect().await
                    };
                    
                    match reconnect_result {
                        Ok(_) => {
                            info!(
                                exchange = exchange_name,
                                "[RECONNECT] Reconnection successful"
                            );
                        }
                        Err(e) => {
                            error!(
                                exchange = exchange_name,
                                error = ?e,
                                "[RECONNECT] Reconnection failed"
                            );
                        }
                    }
                }
            },
            _ = shutdown_rx.recv() => {
                info!(exchange = exchange_name, "Reconnect monitor shutting down");
                break;
            }
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::traits::tests::MockAdapter;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_reconnect_monitor_triggers_on_stale() {
        // Setup mock adapter with stale=true
        let adapter = Arc::new(Mutex::new(MockAdapter::new()));
        adapter.lock().await.set_stale(true);
        
        let config = ReconnectConfig {
            heartbeat_check_interval_secs: 1, // 1s for fast test
        };
        
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let monitor_handle = tokio::spawn({
            let a = adapter.clone();
            reconnect_monitor_task(a, config, shutdown_rx)
        });
        
        // Wait 1.5s pour 1 check cycle
        tokio::time::sleep(Duration::from_millis(1500)).await;
        
        // Verify reconnect was called
        assert_eq!(adapter.lock().await.reconnect_call_count(), 1);
        
        // Cleanup
        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }

    #[tokio::test]
    async fn test_reconnect_monitor_no_trigger_when_healthy() {
        let adapter = Arc::new(Mutex::new(MockAdapter::new()));
        adapter.lock().await.set_stale(false); // Healthy
        
        let config = ReconnectConfig {
            heartbeat_check_interval_secs: 1,
        };
        
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let monitor_handle = tokio::spawn({
            let a = adapter.clone();
            reconnect_monitor_task(a, config, shutdown_rx)
        });
        
        tokio::time::sleep(Duration::from_millis(1500)).await;
        
        // Reconnect should NOT have been called
        assert_eq!(adapter.lock().await.reconnect_call_count(), 0);
        
        let _ = shutdown_tx.send(());
        let _ = monitor_handle.await;
    }

    #[tokio::test]
    async fn test_reconnect_monitor_shutdown() {
        let adapter = Arc::new(Mutex::new(MockAdapter::new()));
        let config = ReconnectConfig::default();
        
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        
        let monitor_handle = tokio::spawn({
            let a = adapter.clone();
            reconnect_monitor_task(a, config, shutdown_rx)
        });
        
        // Trigger shutdown immediately
        let _ = shutdown_tx.send(());
        
        // Monitor should terminate cleanly
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            monitor_handle
        ).await;
        
        assert!(result.is_ok(), "Monitor task should shutdown gracefully");
    }
}
