# Decision timing: the last responsible moment

Use this reference when discussing uncertain architectural choices, premature abstraction,
speculative flexibility, or expensive migrations. It complements Ousterhout's guidance on
design quality with guidance on when to commit. The procedure and software scenarios below
are this skill's synthesis; they are not quotations or examples from the cited authors.

## Concepts and limits

**Last responsible moment (Lean software development).** Mary and Tom Poppendieck describe
delaying commitment until further delay would eliminate an important alternative. Their
later treatment emphasizes making decisions reversible so work can proceed and choices
can change. Apply deferral selectively to costly commitments where learning can help.

**Real Options (Chris Matts and Olav Maassen).** Alternatives have value, expire under
particular conditions, and cost something to preserve and exercise. Identify those
conditions and actively gather information. Earlier commitment can be sensible when its
benefit exceeds the value of waiting. This is a qualitative decision lens here, not a
claim that an architectural choice has a precisely calculable financial option value.

**YAGNI (Extreme Programming; explained by Martin Fowler).** Avoid capabilities and
flexibility built only for presumed future needs. Refactoring and keeping code easy to
change support this approach. A simpler general interface that benefits current callers
does not violate it; unused extension points that complicate today's code need evidence.

These concepts do not justify leaving correctness, data integrity, or known requirements
unresolved in shipped behavior. Unknown requirements and unclear implementation choices
are different problems: clarify required behavior even when the mechanism stays open.

## Applying the concepts

For a consequential choice, consider:

- **Commitment:** What becomes expensive to reverse, and at what point? Publishing a
  contract or accumulating production data can change the answer even if the code is
  currently easy to edit.
- **Alternatives:** Which viable options would waiting preserve or eliminate? Include
  the effort and complexity of keeping them open.
- **Evidence:** What specific observation, prototype, benchmark, or user feedback could
  change the choice? Waiting without a plausible learning path has little value.
- **Expiry:** What event or date requires commitment? Work backward from the constraint
  to include implementation, migration, validation, and appropriate contingency. If the
  timing is unknown, identify how to establish it rather than inventing a deadline.
- **Progress:** What useful work can proceed now without locking in the uncertain choice?
  Prefer existing conventions and cohesive boundaries over speculative frameworks.

Recommend a proportionate action:

| Action | When it fits |
| --- | --- |
| Decide now | Evidence is sufficient, waiting offers little learning, or delay would block work or lose an important option. |
| Choose reversibly | Work needs a choice now and changing it later is inexpensive. State any assumption that could make reversal costly. |
| Defer commitment and learn | Reversal is costly, relevant evidence is forthcoming, and viable alternatives can remain open at an acceptable cost. |

For deliberate deferral, explain the open choice, the next learning action, the commitment
trigger, and what proceeds now. Use the team's existing planning mechanism if tracking is
needed; do not require a decision log or architecture document for every small choice.
Reassess when evidence or constraints change. Resolve the choice when the reason to wait
ends; the principle does not require using every remaining day.

## Choosing a learning technique

*The Pragmatic Programmer* distinguishes prototypes from tracer bullets. Use the
uncertainty to choose the technique; neither is required for every design decision.

| Technique | Useful when | What happens to the code |
| --- | --- | --- |
| Prototype | A focused question can be answered in isolation, such as whether a query meets a workload target. | Treat it as disposable; record the finding and the assumptions that limit it. |
| Tracer bullet | The uncertainty lies in how the real parts fit together. | Build a narrow end-to-end path intended to evolve, with the quality needed for its actual use. |

For example, before choosing a reporting architecture, use representative data in a
disposable query benchmark if query performance is the unknown. If the unknown is how
authorization, request handling, storage, and rendering interact, implement one real
report through that path and use the integration feedback to guide the next increment.
These are original examples, not scenarios from the book.

State what evidence would change the choice and when to stop exploring. A prototype's
success does not establish production readiness, and a tracer bullet need not implement
every feature. Keep required behavior correct for the path actually delivered.

The book's reversibility guidance reinforces the existing recommendation to keep changes
local and reconsider choices as evidence arrives; assess actual reversal cost rather than
assuming every decision can be undone cheaply.

## Worked scenarios

### Defer service extraction while learning

**Request:** A new reporting feature may eventually need independent scaling. Should we
create a separate service now?

**Reasoning:** Independent deployment adds operational and distributed-system costs;
scaling and ownership requirements are still uncertain. A cohesive in-process reporting
module can deliver the feature and provide workload measurements.

**Recommendation:** Keep reporting inside the application for now, with its data access
and reporting logic owned together. Measure representative load and clarify ownership.
Commit to extraction or continued in-process operation before the capacity or deployment
constraint requires it, allowing time for migration and testing. Establish that constraint
with the team. Do not build a generic transport layer merely to preserve the possibility.

### Decide now when waiting would remove an option

**Request:** We ship a public API next week. Can we postpone deciding whether monetary
amounts include a currency until consumers ask for it?

**Reasoning:** The required behavior and consumer needs must be clarified before release;
external adoption makes changing the contract costly. Waiting past publication silently
commits consumers to the initial representation.

**Recommendation:** Resolve the money semantics with consumers now and define the
contract before publication. Defer internal representation choices that remain hidden
and inexpensive to change. The responsible moment precedes the binding external action.

### Choose reversibly instead of designing for hypothetical scale

**Request:** A new internal job could use our existing scheduler or a new workflow engine.
Should we research every engine before implementing it?

**Reasoning:** If the current job has straightforward scheduling needs and no durable
engine-specific state, the existing scheduler may be inexpensive to replace. Research
offers little immediate benefit, and a generic engine abstraction adds maintenance work.

**Recommendation:** Use the existing scheduler and keep job logic separate from scheduling
details. Revisit when a concrete requirement such as durable multi-step recovery appears.
If persisted workflow state or recovery guarantees are already required, reassess the
reversal cost now; this example's assumption no longer holds.

## Interaction with Ousterhout's principles

- **Strategic investment:** Improve today's boundaries and clarity while deferring an
  uncertain commitment. Deferral with a learning action and trigger differs from
  accumulating unexplained TODOs.
- **Information hiding:** Hide an implementation decision so changing it stays local.
  Added wrappers still need to earn their interface cost; optionality alone does not
  make a shallow module useful.
- **General-purpose interfaces:** Favor a simpler interface that serves present needs.
  Assess speculative extension mechanisms by their added complexity and evidence of use.
- **Design it twice:** Compare alternatives through sketches or targeted experiments.
  Implementing and maintaining both alternatives needs stronger justification.

## Sources

- [Dave Thomas and Andy Hunt, The Pragmatic Programmer: official tips](https://pragprog.com/tips/),
  especially tips 18 (reversibility), 20 (tracer bullets), and 21 (prototypes).
- [Jeff Atwood, The Last Responsible Moment (2006)](https://blog.codinghorror.com/the-last-responsible-moment/).
  Accessible introduction quoting *Lean Software Development: An Agile Toolkit* by Mary
  and Tom Poppendieck, and connecting the concept to avoiding premature generalization.
- [Mary and Tom Poppendieck, Implementing Lean Software Development, Chapter 2, p. 32 (2006)](https://res.infoq.com/articles/poppendieck-implementing-lean/en/resources/poppendieck_ch02.pdf).
  Primary-source discussion of irreversible decisions, reversibility, and experimentation.
- [Olav Maassen and Chris Matts, “Real Options” Underlie Agile Practices (2007)](https://www.infoq.com/articles/real-options-enhance-agility/).
  Primary-source account of preserving options, discovering their expiry conditions, and
  gathering information before commitment.
- [Martin Fowler, Yagni (2015)](https://martinfowler.com/bliki/Yagni.html).
  Explains speculative capabilities and abstractions, their costs, and why refactoring
  and maintaining changeability remain necessary.
