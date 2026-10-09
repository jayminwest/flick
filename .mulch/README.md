# .mulch/

This directory is managed by [mulch](https://github.com/jayminwest/mulch) — a structured expertise layer for coding agents.

## Key Commands

- `ml init`      — Initialize a .mulch directory
- `ml add`       — Add a new domain
- `ml record`    — Record an expertise record
- `ml edit`      — Edit an existing record
- `ml query`     — Query expertise records
- `ml prime [domain]` — Output a record index for priming (optionally scoped to one domain)
- `ml show <id>`  — Show a full record
- `ml search`   — Search records across domains
- `ml status`    — Show domain statistics
- `ml validate`  — Validate all records against the schema
- `ml prune`     — Remove expired records

## Structure

- `mulch.config.yaml` — Configuration file
- `expertise/`        — JSONL files, one per domain

## Configuration

Optional knobs in `mulch.config.yaml`:

```yaml
prime:
  default_mode: manifest   # or "full" (the default): a budget-capped record
                           # index, failures first. "manifest" lists per-domain
                           # counts instead. Scoping flags (`--files`,
                           # `<domain>`) always emit records.

search:
  boost_factor: 0.1        # multiplier on BM25 scores for confirmed records.
                           # 0 disables (pure BM25). Override with
                           # `ml search --no-boost`.
```
