# .githooks/

Repository-level git hooks. **Activate them once per clone:**

```bash
git config core.hooksPath .githooks
```

(On Windows, also ensure the files are executable in WSL/Git Bash. The
default checkout permissions usually work; if not, `chmod +x .githooks/*`.)

## Hooks

### `commit-msg`

Rejects any commit message that contains an AI-tool attribution
(Co-Authored-By: Claude, Generated with Claude, 🤖, etc.). The full
pattern list is at the top of the script. CI mirrors the same check on
the server side so a missed local install does not let a trace through.

There is **no `--no-verify` escape valve in policy** — `git commit
--no-verify` works at the git layer, but a CI run will reject the push.
If a legitimate change to the hook is needed, edit `commit-msg` in a
normal commit.
