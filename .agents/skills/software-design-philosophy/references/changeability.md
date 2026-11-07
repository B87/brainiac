# Changeability: ETC, DRY, and orthogonality

Read this for abstraction choices, repeated logic, or changes that spread across unrelated
concerns. Concepts come from Dave Thomas and Andy Hunt's *The Pragmatic Programmer*,
20th Anniversary Edition. The review procedure and examples here are this skill's
synthesis, not examples from the book.

## Easier to Change (ETC)

Use changeability to evaluate a design. Select a plausible change grounded in current
requirements or observed pressure, then trace what must change, what knowledge a maintainer
needs, and which callers are affected. Compare that with the proposed abstraction's cost
today. More extension points do not automatically make a design easier to change.

For example, if an exporter currently writes CSV, consider a concrete requirement to
change the column order. Keeping column selection in one place may simplify that change.
A plugin registry for hypothetical formats adds a different cost and needs its own reason.
If no plausible change distinguishes two designs, prefer the simpler current design rather
than inventing requirements to justify flexibility.

## DRY: identify the knowledge before extracting code

DRY concerns multiple representations of the same knowledge or intent, including rules
spread across code, schemas, and documentation. Textual similarity is only a clue. Ask
whether changing the underlying requirement should change both locations together.

### Same rule, different-looking code

Suppose checkout calculates a subscription's cancellation deadline while a notification
job independently calculates when to send the final cancellation reminder. Both encode
the same cancellation-window policy using different date arithmetic. A policy change
must reach both implementations.

Give the policy a clear owner and let both callers obtain the relevant deadline from it.
Keep notification scheduling separate from the cancellation rule. Independent tests can
still state expected behavior; do not make their expected results call the production
calculation, which would remove their ability to catch a shared mistake.

### Same-looking code, different rules

Suppose an account handle and an invoice reference currently both allow 20 characters.
Their validators look identical, but the product's account policy and an external billing
system own those limits independently. Changing one need not change the other.

Keep those policies separate. Share a low-level validation mechanism only if it simplifies
current use without combining policy ownership. Avoid a single shared limit whose name
obscures why it exists, or a helper with switches for unrelated rules.

### Choosing a remedy

Establish the shared requirement before proposing extraction. Depending on the source of
truth, an owned function, data model, schema, or generated representation may be enough.
If two representations are deliberately retained, such as a cache or generated client,
make their synchronization explicit rather than pretending no duplication exists.

## Orthogonality

Unrelated concerns should be able to change independently. To assess this, trace a change
through dependencies and shared mutable state. Which additional edits or coordination
does it require, and are those consequences inherent to the requirement?

For example, a report renderer that chooses a database transaction isolation level ties
presentation to persistence policy. Move that decision to the owner of data access, then
check that the renderer can change format without changing transaction behavior. Existing
cohesive boundaries may already provide enough separation; an extra interface is optional.

Orthogonality complements Ousterhout's information hiding. It is not a demand for one
class per concern or zero communication between modules. Keep information that must
change together cohesive, and avoid wrappers that merely pass calls through.

## Sources

- [The Pragmatic Programmer: official tips](https://pragprog.com/tips/), especially
  tips 14 (changeability) and 17 (independence between unrelated concerns).
- [The Evils of Duplication: official book excerpt](https://media.pragprog.com/titles/tpp20/dry.pdf).
  Explains DRY as knowledge and intent, including why similar code may represent
  independent requirements.
