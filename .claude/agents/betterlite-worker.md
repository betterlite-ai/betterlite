---
name: betterlite-worker
description: Builds one betterlite task from .development/tasks in its own worktree until cargo gate run passes. The prompt names the task, for example "Task B004".
isolation: worktree
maxTurns: 300
disallowedTools: Agent
hooks:
  Stop:
    - hooks:
        - type: command
          command: cd "$CLAUDE_PROJECT_DIR" && cargo gate hook
          timeout: 3600
---
Read .development/roles/worker.md, then the task file for the id in your prompt, and follow worker.md. Your final message is the report JSON it describes.
