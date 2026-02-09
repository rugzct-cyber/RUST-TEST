---
description: Pull latest code from GitHub, rebuild, and restart the bot on VPS
---

# Update Bot on VPS

// turbo-all

1. Connect to the VPS via SSH:
```
ssh -tt -o StrictHostKeyChecking=no root@5.161.53.11
```
Password: `Hft$Bot2026!Vps`

2. If the bot is running, first stop it: attach to tmux (`tmux attach -t bot`), press `Ctrl+C`, then open a new window `Ctrl+B` then `C`.

3. Pull the latest code from GitHub:
```
cd /root/bot5 && git pull
```

4. Rebuild the bot in release mode:
```
cd bot && source $HOME/.cargo/env && cargo build --release 2>&1 | tail -5
```

5. Relaunch the bot (switch to tmux window 0 with `Ctrl+B` then `0`):
```
cd /root/bot5/bot && export $(grep -v '^#' .env | grep -v '^\s*$' | xargs) && ./target/release/hft_bot
```
