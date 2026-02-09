---
description: Stop the HFT bot on the VPS
---

# Stop Bot on VPS

// turbo-all

1. Connect to the VPS via SSH:
```
ssh -tt -o StrictHostKeyChecking=no root@5.161.53.11
```
Password: `Hft$Bot2026!Vps`

2. Attach to the tmux session:
```
tmux attach -t bot
```

3. Switch to the bot window if needed: `Ctrl+B` then `0`.

4. Press `Ctrl+C` to stop the bot.

5. Optionally kill the tmux session entirely:
```
tmux kill-session -t bot
```
