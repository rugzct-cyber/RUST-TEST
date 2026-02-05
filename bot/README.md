# HFT Arbitrage Bot - MVP

Bot d'arbitrage delta-neutral entre Vest et Paradex.

## Structure

```
bot4/
├── Cargo.toml           # Dépendances minimales
├── src/
│   ├── main.rs          # Point d'entrée (scaffold)
│   ├── lib.rs           # Module racine
│   ├── error.rs         # Types d'erreur
│   ├── adapters/        # Connexion exchanges (230KB)
│   │   ├── vest.rs      # Adapter Vest + EIP-712 signing
│   │   ├── paradex.rs   # Adapter Paradex + Starknet signing
│   │   ├── traits.rs    # Interface ExchangeAdapter
│   │   └── types.rs     # Orderbook, Order, Fill
│   ├── core/            # Logique métier
│   │   ├── spread.rs    # Calcul entry/exit spread
│   │   ├── vwap.rs      # VWAP orderbook
│   │   ├── state.rs     # État applicatif
│   │   └── channels.rs  # Communication inter-tâches
│   └── config/          # Configuration YAML
│       ├── types.rs     # BotConfig, AppConfig
│       └── loader.rs    # Chargement YAML
```

## Lancer

```bash
cd c:\Users\jules\Documents\bot4
cargo run
```

## Prochaines étapes

1. **Créer config.yaml** avec credentials Vest/Paradex
2. **Implémenter connexion** dans main.rs
3. **Ajouter boucle de polling** orderbooks
4. **Calculer spreads** et logger les opportunités

## Origine

Extrait de `bot3/y/bot` - seuls les modules essentiels ont été conservés.
