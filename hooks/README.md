# kern Claude Code hooks

Two Claude Code hooks drive kern's automatic memory. They are plain Node ESM
scripts with no dependencies, and both **fail open** — any error exits 0 and the
session proceeds untouched.

| Hook | Event | What it does |
|------|-------|--------------|
| `kern-capture.mjs` | `Stop` | Extracts the new conversation delta from the transcript and writes it to `<cwd>/.kern/capture/`. The daemon drains and distills it. |
| `kern-recall.mjs` | `SessionStart` | Reads `<cwd>/.kern/digest.md` and injects it into the new session as context. |

Both are **project-scoped by a guard**: `kern-capture` no-ops in any directory
without a `.kern/` folder, so a single global registration is safe across every
project — only directories where a kern is (or has been) active get captured.

## Install

Register both once in `~/.claude/settings.json`. Use absolute paths to your
checkout. Example (adjust the node path and the repo path for your machine):

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "node \"/abs/path/to/kern/hooks/kern-recall.mjs\""
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "node \"/abs/path/to/kern/hooks/kern-capture.mjs\""
          }
        ]
      }
    ]
  }
}
```

On Windows, if `node` is not on `PATH` for hook execution, use the full path,
e.g. `"\"C:/Program Files/nodejs/node.exe\" \"C:/path/to/kern/hooks/kern-recall.mjs\""`.

If you already have `SessionStart` or `Stop` arrays, append these entries rather
than replacing them — multiple hooks per event are allowed.

## Verify

After editing, confirm the JSON still parses:

```bash
node -e "JSON.parse(require('fs').readFileSync(process.env.HOME + '/.claude/settings.json','utf8')); console.log('ok')"
```

Then open a new Claude Code session in a project with a `.kern/` folder. After
the daemon has written a digest, the recall hook injects it at the next
`SessionStart`; the capture hook spools each session's delta on `Stop`.
