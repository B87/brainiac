---
title: Payments service
tags: [project]
---

# Payments service

Context for the [payments-api](https://example.com/org/payments-api) repository.

## Decisions

- Use idempotency keys on every write.
- Retries back off exponentially (`fetch_with_backoff`).

## Next steps

- [ ] Review the migration plan
- [x] Draft the rollout note

| Env | URL |
| --- | --- |
| staging | https://staging.example.com |

```bash
cargo test --workspace
```

See also [[Rollout checklist]].
