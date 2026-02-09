# VPS Bot - Commandes Rapides
# IP: 5.161.53.11 | MDP: Hft$Bot2026!Vps

## ============ CONNEXION ============

# Se connecter au VPS
ssh root@5.161.53.11

## ============ BOT ============

# Retrouver le bot qui tourne déjà (TUI)
tmux attach -t bot

# Lancer le bot (nouvelle session)
tmux new -s bot
cd /root/bot5/bot && export $(grep -v '^#' .env | grep -v '^\s*$' | xargs) && ./target/release/hft_bot

# Arrêter le bot
Ctrl+C

# Partir sans arrêter le bot (il continue de tourner)
Ctrl+B puis D

## ============ CONFIG ============

# Modifier le fichier de config trading
nano /root/bot5/bot/config.yaml

# Modifier les variables d'environnement (clés API)
nano /root/bot5/bot/.env

# Sauvegarder dans nano: Ctrl+O → Enter → Ctrl+X

## ============ MISE A JOUR ============

# Mettre à jour le code depuis GitHub
cd /root/bot5 && git pull

# Recompiler le bot
cd /root/bot5/bot && source $HOME/.cargo/env && cargo build --release

## ============ TMUX ============

# Ouvrir un 2ème onglet (sans arrêter le bot)
Ctrl+B puis C

# Revenir à l'onglet du bot
Ctrl+B puis 0

# Supprimer une session tmux
tmux kill-session -t bot

# Voir les sessions tmux actives
tmux ls

## ============ LATENCE ============

# Tester la latence vers Vest
curl -o /dev/null -s -w "Latency: %{time_total}s\n" https://api.vest.exchange/v1/exchange/info

# Tester la latence vers Paradex
curl -o /dev/null -s -w "Latency: %{time_total}s\n" https://api.prod.paradex.trade/v1/system/time
