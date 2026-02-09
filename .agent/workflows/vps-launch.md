---
description: Launch the HFT bot on the VPS in a new tmux session with TUI
---

# Launch Bot on VPS

// turbo-all

1. Connect to the VPS via SSH:
```
ssh -tt -o StrictHostKeyChecking=no root@5.161.53.11
```
Password: `Hft$Bot2026!Vps`

2. Start a new tmux session and launch the bot:
```
tmux new -s bot
```

3. Load environment variables and start the bot:
```
cd /root/bot5/bot && export $(grep -v '^#' .env | grep -v '^\s*$' | xargs) && ./target/release/hft_bot
```

4. The bot should now be running with the TUI visible. To detach and leave it running: press `Ctrl+B` then `D`.
