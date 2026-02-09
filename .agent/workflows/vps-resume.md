---
description: Resume/reattach to the running bot on the VPS via tmux
---

# Resume Bot on VPS

// turbo-all

1. Connect to the VPS via SSH:
```
ssh -tt -o StrictHostKeyChecking=no root@5.161.53.11
```
Password: `Hft$Bot2026!Vps`

2. Reattach to the existing tmux session where the bot is running:
```
tmux attach -t bot
```

3. If the tmux session doesn't exist (bot was stopped), use `/vps-launch` instead to start a new session.

4. To switch between tmux windows: `Ctrl+B` then `0` (bot) or `1` (shell).
