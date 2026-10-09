# Bridge parser fixtures (synthetic)

These files are SYNTHETIC. No agent was run to make them. They follow the documented
output shapes:

- `claude_result.json`, `claude_error.json`: `claude -p --output-format json` (one
  `{"type":"result",...}` object), from `claude --help` and the Claude Code docs.
- `pi_events.jsonl`, `pi_error.jsonl`: `pi -p --mode json` (a `session` header, then
  agent events), from pi's `docs/json.md` and its `AssistantMessage` type.

Replace them with real captured outputs (tiny prompts, personal data stripped) when the
user runs the spike in plan pl-a32c step 3. Agents must never run claude or pi to do it.
