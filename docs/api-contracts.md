# HFT Arbitrage Bot - API Contracts

> **Updated:** 2026-02-04  
> **Source:** Exhaustive scan of trait interfaces and adapter implementations

---

## ExchangeAdapter Trait (`adapters/traits.rs`)

The core interface all exchange adapters must implement:

```rust
#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    /// Connect to the exchange (WebSocket + auth)
    async fn connect(&mut self) -> ExchangeResult<()>;
    
    /// Disconnect gracefully
    async fn disconnect(&mut self) -> ExchangeResult<()>;
    
    /// Subscribe to orderbook updates for a symbol
    async fn subscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()>;
    
    /// Unsubscribe from orderbook updates
    async fn unsubscribe_orderbook(&mut self, symbol: &str) -> ExchangeResult<()>;
    
    /// Place an order (returns immediately, no polling)
    async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse>;
    
    /// Cancel an existing order
    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()>;
    
    /// Get cached orderbook snapshot
    fn get_orderbook(&self, symbol: &str) -> Option<&Orderbook>;
    
    /// Check if connected
    fn is_connected(&self) -> bool;
    
    /// Check if data is stale (heartbeat timeout)
    fn is_stale(&self) -> bool;
    
    /// Sync orderbooks from internal reader state
    fn sync_orderbooks(&mut self);
    
    /// Reconnect (refresh auth, resubscribe)
    async fn reconnect(&mut self) -> ExchangeResult<()>;
    
    /// Get current position for symbol
    async fn get_position(&self, symbol: &str) -> ExchangeResult<Option<PositionInfo>>;
    
    /// Exchange name for logging
    fn exchange_name(&self) -> &'static str;
}
```

---

## Vest Markets API

### REST Endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/v1/orders` | Place order |
| DELETE | `/v1/orders/{id}` | Cancel order |
| GET | `/v1/orders` | List orders |
| GET | `/v1/positions` | Get positions |
| GET | `/v1/account` | Account info |

### WebSocket Channels

| Channel | Format | Description |
|---------|--------|-------------|
| `orderbook.{symbol}` | `{"channel":"orderbook.BTC-PERP","data":{...}}` | L2 orderbook |
| `trades.{symbol}` | `{"channel":"trades.BTC-PERP","data":[...]}` | Trade stream |

### Authentication (EIP-712)

```rust
pub struct VestConfig {
    pub primary_addr: String,     // Wallet address
    pub primary_key: String,      // Private key (signing)
    pub signing_key: String,      // Subaccount signing key
    pub testnet: bool,
}

impl VestConfig {
    pub fn from_env() -> Result<Self, String>;
}
```

**Required Environment Variables:**
```bash
VEST_PRIMARY_ADDR=0x...
VEST_PRIMARY_KEY=0x...
VEST_SIGNING_KEY=0x...
VEST_TESTNET=false  # Optional, defaults to false
```

### Order Request Format

```json
{
  "client_order_id": "uuid-v7",
  "symbol": "BTC-PERP",
  "side": "buy",
  "order_type": "limit",
  "price": "50000.00",
  "quantity": "0.01",
  "time_in_force": "ioc",
  "reduce_only": false,
  "post_only": false
}
```

---

## Paradex API

### REST Endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| POST | `/v1/orders` | Place order |
| DELETE | `/v1/orders/{id}` | Cancel order |
| GET | `/v1/orders` | List orders |
| GET | `/v1/positions` | Get positions |
| POST | `/v1/onboarding` | Account onboarding |

### WebSocket Channels

| Channel | Format | Description |
|---------|--------|-------------|
| `orderbook.{market}` | Snapshot + deltas | L2 orderbook |
| `orders.{market}` | Order updates | Fill notifications |
| `fills.{market}` | Trade fills | Execution confirmations |

### Authentication (Starknet SNIP-12)

```rust
pub struct ParadexConfig {
    pub private_key: String,       // Starknet private key
    pub account_address: String,   // Starknet account
    pub testnet: bool,
}

impl ParadexConfig {
    pub fn from_env() -> Result<Self, String>;
}
```

**Required Environment Variables:**
```bash
PARADEX_PRIVATE_KEY=0x...
PARADEX_ACCOUNT_ADDRESS=0x...
PARADEX_TESTNET=false  # Optional
```

### Order Request Format

```json
{
  "client_id": "uuid-v7",
  "market": "BTC-USD-PERP",
  "side": "BUY",
  "type": "LIMIT",
  "price": "50000.00",
  "size": "0.01",
  "time_in_force": "IOC",
  "reduce_only": false,
  "post_only": false
}
```

---

## Internal Channel Contracts

### SpreadOpportunity Channel

```rust
// mpsc::channel<SpreadOpportunity> with capacity 1
pub struct SpreadOpportunity {
    pub direction: SpreadDirection,
    pub entry_spread: f64,
    pub vest_ask: f64,
    pub vest_bid: f64,
    pub paradex_ask: f64,
    pub paradex_bid: f64,
    pub timestamp: u64,
}
```

**Producer:** `monitoring_task`  
**Consumer:** `execution_task`

### Shutdown Channel

```rust
// broadcast::channel<()> with capacity 1
```

**Producer:** SIGINT handler in `main.rs`  
**Consumers:** All async tasks

---

## SharedOrderbooks Contract

```rust
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;
```

**Writers:** WebSocket reader tasks (per adapter)  
**Readers:** `monitoring_task` (40Hz polling)

Key access pattern:
```rust
// Read (non-blocking)
let orderbooks = shared.read().await;
let ob = orderbooks.get("BTC-PERP");

// Write (exclusive)
let mut orderbooks = shared.write().await;
orderbooks.insert(symbol, orderbook);
```

---

## Error Contract

All adapter methods return `ExchangeResult<T>`:

```rust
pub type ExchangeResult<T> = Result<T, ExchangeError>;

pub enum ExchangeError {
    ConnectionFailed(String),
    AuthenticationFailed(String),
    OrderRejected { reason: String, details: Option<String> },
    InsufficientBalance { required: f64, available: f64 },
    RateLimited { retry_after_ms: u64 },
    Timeout { operation: String },
    ParseError(String),
    NetworkError(String),
    InvalidSymbol(String),
    PositionNotFound(String),
}
```

---

## Symbol Mapping

| Internal Symbol | Vest Symbol | Paradex Symbol |
|-----------------|-------------|----------------|
| BTC-PERP | BTC-PERP | BTC-USD-PERP |
| ETH-PERP | ETH-PERP | ETH-USD-PERP |
| SOL-PERP | SOL-PERP | SOL-USD-PERP |

Conversion logic in `main.rs`:
```rust
let vest_symbol = bot.pair.to_string();  // "BTC-PERP"
let paradex_symbol = format!("{}-USD-PERP",
    bot.pair.to_string().split('-').next().unwrap_or("BTC")
);  // "BTC-USD-PERP"
```
