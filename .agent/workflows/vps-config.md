---
description: Edit config.yaml or .env on the VPS
---

# Edit Bot Configuration on VPS

// turbo-all

1. Connect to the VPS via SSH:
```
ssh -tt -o StrictHostKeyChecking=no root@5.161.53.11
```
Password: `Hft$Bot2026!Vps`

2. To edit the trading configuration:
```
nano /root/bot5/bot/config.yaml
```

3. To edit environment variables (API keys, etc.):
```
nano /root/bot5/bot/.env
```

4. Save with `Ctrl+O` → Enter, then exit with `Ctrl+X`.

5. **Important**: After editing config, you need to restart the bot for changes to take effect. Stop the bot with `Ctrl+C` in tmux, then relaunch:
```
cd /root/bot5/bot && export $(grep -v '^#' .env | grep -v '^\s*$' | xargs) && ./target/release/hft_bot
```
