---
name: software-design-philosophy
description: Software design philosophy and vocabulary based on John Ousterhout's "A Philosophy of Software Design," centered on managing complexity. Use during code reviews, architecture/API design, module decomposition, refactoring, and error-handling design. Trigger when the user asks whether design is too complex, how to split modules, reduce coupling, improve interfaces, handle errors, name things well, or whether a design makes sense — even without using book terms. Also trigger on getLine/putLine APIs, split/join at call sites, UI-named storage methods (backspace, deleteSelection), APIs mirroring internal structures, deep vs shallow modules, information leakage, or feedback on structure, boundaries, and naming beyond bug fixes.
---

# Software Design Philosophy

Distilled from John Ousterhout's *A Philosophy of Software Design*. Central thesis: **the
core challenge of software design is managing complexity**, and complexity is created by
the accumulation of many small decisions, not a single catastrophic one. That's why the
right mindset is zero-tolerance — treat "minor" complexity in a name, an interface, or an
error path as worth fixing, because it never stays minor once it compounds.

Use this skill to bring vocabulary and a diagnostic lens to design conversations — naming
symptoms precisely (shallow module, information leakage, pass-through method) is often
more useful than a vague "this feels off." If a repo already has its own code-review or
simplification tooling, this skill supplies the conceptual framing underneath it; it
doesn't replace mechanical diff-checking.

## How to apply this

1. **Diagnose before prescribing.** Read the code/design in question and ask which of the
   three complexity symptoms (below) it produces. Naming the symptom is usually more
   useful to the user than jumping straight to a fix.
2. **Reach for the Red Flags table** (Section 14) as a fast checklist during code review —
   it's the compressed version of everything below.
3. **Explain the "why," not just the rule.** Ousterhout's ideas land better as reasoning
   ("this interface forces every caller to know about an implementation detail") than as
   a citation ("rule #4 says..."). Adapt the framing to the code in front of you rather
   than quoting the book.
4. **Weigh trade-offs, don't dogma them.** Depth, generality, and pulling complexity
   downward are usually right, but a smaller/uglier special-case solution is sometimes the
   pragmatic choice under real deadlines. Say so when it applies — the goal is better
   judgment, not rule-following.
5. **Reach for a worked example when it'll land better than another abstract description.**
   `references/examples.md` has concrete illustrations from the book — pull one in if the
   user is going deeper, seems unconvinced, or asks for an example. Don't dump them by
   default.
6. **Match the example to the symptom** — read only the relevant section of
   `references/examples.md`:

   | If the conversation is about… | Pull… |
   | --- | --- |
   | "It works / passes tests — is the design fine?" | text-editor class study |
   | API mirrors storage (`getLine`/`putLine`), callers split/join, UI methods on a data class | text-editor Text class API |
   | Interface too wide for what it does | Unix I/O vs Java streams |
   | Errors/cases forced on every caller | Unix unlink vs Windows delete; Java checked exceptions |
   | Hard internal choice exposed as config or caller duty | hash map resizing |

---

## 1. Complexity: know the enemy

Complexity is anything about a system's structure that makes it hard to understand or
modify. It's independent of raw size — a small system can be tangled, and a large one can
stay simple if it's decomposed well.

**Three symptoms**, useful for diagnosing *why* something feels hard to work with:

- **Change amplification** — a conceptually simple change requires edits in many places.
- **Cognitive load** — a developer has to hold a lot of information in their head to make
  a safe change. Note: fewer lines isn't automatically simpler — sometimes *more* explicit
  code is simpler because it removes the need to infer hidden behavior.
- **Unknown unknowns** — it's not even clear what needs to change or what information is
  relevant. This is the most dangerous symptom, because it means mistakes won't be
  obvious until they cause failures.

**Two root causes**, useful for diagnosing *where* complexity comes from:

- **Dependencies** — code that can't be understood or changed in isolation because it's
  entangled with other code.
- **Obscurity** — important information isn't obvious: vague names, missing context,
  implicit conventions, unstated constraints.

Because complexity accumulates from many small decisions rather than one big one, treat
every instance — even ones that seem trivial in isolation — as worth calling out.

## 2. Strategic vs. tactical programming

**Tactical programming** optimizes for "get this feature working now" and defers design
to "later" (which rarely comes). It's fast in the moment and compounds into a system that
gets harder to change every week.

**Strategic programming** treats good design as an equal goal alongside working code, with
a standing investment of roughly 10–20% of development time into design improvement —
looking for a chance to improve the surrounding design with *every* change, rather than
treating "make it pass" as the finish line. The unit of progress isn't a shipped feature;
it's a clean abstraction.

When reviewing code or a plan, it's fair to flag when something looks like the tactical
shortcut (a special case bolted on, a TODO instead of a decision) versus a strategic
investment.

## 3. Deep modules — the most important idea here

Picture a module as a rectangle: its **width** is the complexity of its interface, and its
**depth** is how much functionality is hidden behind that interface.

- **Deep module** (good): simple interface, a lot of useful functionality behind it.
  Example: Unix file I/O — five syscalls (`open`, `read`, `write`, `lseek`, `close`) hide
  an entire filesystem's worth of complexity.
- **Shallow module** (red flag): the interface is nearly as complicated as what it does.
  Example: Java's I/O streams, where reading a file means composing
  `FileInputStream` + `BufferedInputStream` + `ObjectInputStream` just to get started.

Design the interface so the *common* case is trivial to use, even if that means the
implementation has to do more work internally, or rare cases require a more elaborate
call. A simple interface is worth more than a simple implementation — and the common path
should never have to pay for the existence of rare ones.

*(For why "it works" isn't the same question as "it's well designed," the text-editor
class study in `references/examples.md` is the sharpest illustration.)*

## 4. Information hiding and information leakage

Each module should encapsulate a design decision — a data structure, an algorithm, a
low-level mechanism, a policy — behind an interface that doesn't expose it. That's what
actually reduces dependencies between modules; it's not just about tidiness.

**Information leakage** (red flag) is when the *same* design decision is reflected in more
than one place — two modules that both need to change if that decision changes, even
though only one "owns" it. A common source is **temporal decomposition**: splitting code
into modules based on the order operations happen in, rather than based on what
information each one needs to hide. Steps that just happen to run sequentially often end
up sharing knowledge they shouldn't.

To fix leakage: merge the modules that share the knowledge, or — if that's not practical —
concentrate the shared information behind one new, deep module that both can depend on
instead of directly on each other.

*(When a storage class takes UI types like `Cursor` or `Selection`, see the text-editor
`Text` class example in `references/examples.md`.)*

## 5. General-purpose vs. special-purpose modules

General-purpose modules tend to be deeper, because a general interface can satisfy more
use cases with fewer, more powerful methods than a narrowly-tailored one needs.

When designing something new, ask: *what's the most general-purpose interface that still
satisfies what I need right now?* That's a question about the **interface**, not the
implementation — the implementation should still only do what's actually needed today;
don't over-build behind a general API just because the API is general.

Keep general-purpose and special-purpose code cleanly separated rather than mixed in the
same module — mixing them is its own red flag (see Section 14).

*(The text-editor `Text` class API in `references/examples.md` contrasts
`backspace(Cursor)`/`getLine`/`putLine` with a character-oriented
`insert`/`delete`/`changePosition` design — same project as the class study, but focused
on interface shape.)*

## 6. Different layers, different abstractions

A system's layers should each contribute a genuinely different abstraction. If two
adjacent layers look similar, something is probably wrong with how responsibility is
split between them.

Two concrete tells:

- **Pass-through method** (red flag) — a method that does almost nothing but forward its
  arguments to another method with a near-identical signature. It signals the layers
  aren't actually offering different abstractions.
- **Pass-through variable** (red flag) — a parameter threaded through several layers of
  calls that don't themselves use it, just to reach a layer deep down that does. Consider
  a context object, dependency injection, or reshaping the module boundaries so the
  variable doesn't need to travel so far.

## 7. Pull complexity downward

When complexity can't be eliminated, the module — not its callers — should absorb it.
Most modules have far more call sites than they have maintainers, so it's a better trade
for the few people touching the implementation to deal with hard cases than for every
caller to have to.

Two common anti-patterns that do the opposite:

- Turning a hard internal decision into a configuration parameter and handing it to
  whoever deploys the system.
- Throwing an exception for a condition the module itself could have resolved, and
  making every caller handle it.

Both look like they save implementation effort, but they multiply complexity across every
place that has to deal with the consequence.

*(The line-oriented vs character-oriented `Text` API in `references/examples.md` is the
canonical "pull complexity downward" illustration for API design. Java's checked
exceptions and hash-map resizing there are good contrasting examples too — one pushes
complexity up, the other pulls it down.)*

## 8. Define errors out of existence

Exception and error handling is one of the largest sources of complexity in most systems,
so reducing how many error paths exist — not just handling them more gracefully — is one
of the highest-leverage design moves available.

Ways to do this:

- **Redefine the semantics** so the error condition simply can't occur. E.g., a
  `delete(key)`-type operation that succeeds whether or not the key existed, because its
  actual contract is "guarantee the key is gone afterward," not "the key must have been
  present." Or a `substring(start, end)` that clips out-of-range indices instead of
  throwing.
- **Exception masking** — catch and resolve the condition at a low level so it never
  surfaces to callers at all.
- **Exception aggregation** — handle a whole family of related exceptions in one place
  instead of scattering `try/catch` at every call site that might trigger one of them.

This isn't about swallowing errors that matter — a dropped network packet still needs
real handling. It's about noticing which "errors" are only errors because of an
arbitrary interface decision, and removing that decision.

*(Unix `unlink` vs. Windows file deletion, in `references/examples.md`, is a good real-world
contrast of two systems making opposite calls on the same operation.)*

## 9. Design it twice

For any design decision worth deliberating over, sketch at least two genuinely different
approaches before picking one — even when the first idea already looks good. Compare them
on interface simplicity, generality, performance, and how hard each is to implement. The
point isn't ceremony; it's that the second option, even a bad one, usually reveals
something about the first that wasn't visible when it had no competition.

## 10. The philosophy of comments

Comments exist to capture design intent and decisions that the code itself can't express
— they're part of the abstraction, not an afterthought. A good interface comment means
callers don't need to read the implementation to use it correctly. Writing the comment
*before* the implementation is a design tool: if a clear comment won't come, that's often
a sign the design isn't settled yet, not that the comment is hard to write.

What comments should carry: the *why*, constraints, edge-case behavior, side effects —
anything true about the code that isn't visible just by reading it.

**Comment repeats code** (red flag): a comment that's just an English transliteration of
the line beneath it adds no information and is dead weight the moment the code changes.

Three layers worth distinguishing:

- **Interface comments** — what a piece of code does and why, with no implementation
  detail leaking in.
- **Implementation comments** — how and why it's built this particular way internally.
- **Cross-module comments** — a decision or dependency that spans a module boundary and
  wouldn't be visible from inside any single module.

## 11. The art of naming

A good name is precise (unambiguous), consistent (always means the same thing across the
codebase), and informative enough to act as documentation on its own.

Two red flags:

- **Vague name** — generic placeholders like `data`, `result`, `tmp`, `info` that carry no
  real information about what the thing is.
- **Hard to name** — if you (or the user) can't find a precise, intuitive name for
  something after genuinely trying, that's often a signal the *entity itself* is
  ill-defined, not just a naming problem. Suggest reconsidering what the thing actually is
  before reaching for a better label.

## 12. Consistency

Handle the same kind of thing the same way everywhere — naming, style, interface
patterns, invariants. Consistency is what lets a pattern learned once apply everywhere
else, which is a direct reduction in cognitive load.

It's not worth breaking an established convention for a marginally "better" one unless the
improvement is big enough to justify a full migration — a codebase with two competing
conventions is usually worse than one with a single mediocre one.

## 13. Code should be obvious

The test of good design is that a reader can understand what code does and why without
much effort. Obviousness comes from consistent naming, deliberate whitespace/structure,
comments that fill in what the code can't say, and avoiding implicit control flow (e.g.
surprising callback/event chains) unless the paradigm genuinely calls for it.

**Non-obvious code** (red flag): if a competent reader can't tell what a piece of code
does or why without asking someone, it has failed this test — regardless of how "clean" it
looks syntactically.

## 14. Red flags quick reference

Use this as a fast checklist during review — each row names a smell precisely enough to
say out loud:

| Signal | Meaning |
| --- | --- |
| Shallow Module | Interface is nearly as complex as what it implements |
| Information Leakage | The same design decision is reflected in multiple modules |
| Temporal Decomposition | Modules split by execution order, not by information hiding |
| Overexposure | The common API path forces callers to know about rarely-used features |
| Pass-Through Method | A method just forwards its args to another method with a similar signature |
| Pass-Through Variable | A parameter threaded through layers that never use it |
| Repetition | Non-trivial logic duplicated across locations |
| Special-General Mixture | General-purpose and special-purpose code aren't cleanly separated |
| Conjoined Methods | You can't understand one method without also understanding another |
| Comment Repeats Code | The comment is just an English translation of the code beneath it |
| Impl. Contaminates Interface | Interface docs expose implementation detail callers don't need |
| Vague Name | A name too generic to convey real information (`data`, `tmp`, `result`) |
| Hard to Pick a Name | Can't find a precise name — often means the design itself is unclear |
| Hard to Describe | A module needs a long comment to be fully explained — likely too complex |
| Non-Obvious Code | A reader can't easily tell what the code does or why |

## 15. Design principles summary

- Complexity is incremental — sweat the small stuff.
- Working code by itself isn't the goal; design quality is too.
- Make continual small investments in design (~10–20% of time), not a big refactor later.
- Modules should be deep: simple interface, real functionality hidden behind it.
- The common usage path should be the simplest one, even at the cost of a more complex
  implementation.
- General-purpose modules are usually deeper than special-purpose ones — but keep the two
  kinds of code cleanly separated.
- Adjacent layers should offer genuinely different abstractions, not near-duplicates.
- Push complexity down into the module, not out onto every caller.
- Define errors (and other special cases) out of existence where the semantics allow it.
- Design it twice before committing to the first idea.
- Comments exist to say what the code can't — never to restate it.
- Names should be precise, consistent, and informative; software should be designed for
  ease of reading, not ease of writing.
- The real unit of progress is a good abstraction, not a shipped feature.
