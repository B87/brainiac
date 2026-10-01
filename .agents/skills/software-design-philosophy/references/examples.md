# Worked examples from the book

These are concrete illustrations drawn from *A Philosophy of Software Design*. Reach for
one when a live example would land better than another round of abstract description —
especially if the user is asking to go deeper, seems skeptical of a concept, or explicitly
asks "what's an example of that." Don't recite these unprompted in every response; they're
here to be pulled in, not dumped.

| Example | Best for |
| --- | --- |
| Text-editor class study | Strategic vs tactical; "working ≠ well-designed" |
| Text-editor Text class API | General-purpose API, line vs character interface, information leakage |
| Unix I/O vs Java streams | Deep vs shallow modules |
| Unix unlink vs Windows delete | Defining errors out of existence |
| Java checked exceptions | Pushing complexity to callers |
| Hash map resizing | Pulling complexity downward |

## The text-editor class study (why "working code isn't enough")

Ousterhout's Stanford course had students independently build a GUI text editor from
scratch, from the same spec, across several week-long assignments. Comparing the
submissions side by side is where the book's central claim comes from:

- **Line count was a bad predictor of quality.** Some of the longer solutions were far
  easier to work with than some of the shorter, denser ones — density often meant hidden
  assumptions and special cases rather than genuine simplicity.
- **The quality gap was invisible from the tests passing.** All the editors worked. The
  difference only showed up on the *next* assignment, when students had to add a new
  feature to their own editor: the well-designed ones took a small, localized change,
  and the poorly-designed ones needed a painful rewrite of code they'd written themselves
  weeks earlier.

This is the example to reach for when someone is inclined to treat "it passes review /
tests / works" as equivalent to "the design is fine" — the whole point is that those are
different questions, and only one of them is checked by default.

## Text editor Text class API (line-oriented vs character-oriented)

Same Stanford CS190 editor project, but zoomed in on one design decision: how the `Text`
class exposes mutations.

**Three common mistakes:**

1. **UI-specialized API** — `backspace(Cursor)`, `delete(Cursor)`,
   `deleteSelection(Selection)`. Shallow methods tied to keyboard semantics; `Text`
   leaks UI types and concepts.
2. **Line-oriented API** — `getLine(n)`, `putLine(n)` (sometimes `splitLine` /
   `joinLine`). Callers must split and join strings whenever they insert or delete in
   the middle of a line.
3. **Mirroring internal storage in the interface** — storing lines internally is fine;
   exposing lines in the public API is the mistake. Interface and implementation should
   differ.

**Better: character-oriented, general-purpose API:**

```java
void insert(Position position, String newText);
void delete(Position start, Position end);
Position changePosition(Position position, int numChars);
```

`Position` is a neutral document coordinate (not a UI `Cursor`). `changePosition`
crosses line boundaries internally. UI operations compose from primitives:

```java
text.delete(cursor, text.changePosition(cursor, 1));           // Delete key
text.delete(text.changePosition(cursor, -1), cursor);        // Backspace
text.delete(selectionStart, selectionEnd);                   // Delete selection
```

Line-boundary logic, merge/split, and representation details stay inside `Text`. UI code
is slightly more verbose per call site, but the system has fewer methods, less coupling,
and reusable text operations.

Reach for this when callers are doing bookkeeping the module should own (split/join
lines, mirror data-structure shape in the API, or bake UI semantics like "backspace"
into a storage class). Also when debating general-purpose vs special-purpose method
names on the same class.

## Unix file I/O vs. Java streams (deep vs. shallow)

Already the canonical example for Section 3: five Unix syscalls (`open`, `read`, `write`,
`lseek`, `close`) sit in front of an entire filesystem — inodes, block allocation,
caching, journaling — while reading a file in classic Java means composing
`FileInputStream` + `BufferedInputStream` + `ObjectInputStream` just to get moving. Same
underlying capability, very different interface-to-power ratio.

## Unix `unlink` vs. Windows delete (defining an error out of existence)

Unix lets a process delete (`unlink`) a file that another process currently has open —
the OS keeps the file's data alive via reference counting until the last open handle
closes, so "delete a file that's in use" was designed to not be a special case at all.
Windows historically locks open files, so deleting one out from under a running process
fails with a sharing violation — pushing that failure mode onto every user and every
tool, forever (anyone who's hit "this file is open in another program" has felt this).

Useful for contrasting two real systems that made opposite interface decisions about the
exact same operation, with very different consequences for how much everyone downstream
has to think about it.

## Java's checked exceptions (the opposite of pulling complexity downward)

A checked exception thrown three layers down forces every intervening layer to either
handle it or add it to their own `throws` clause — whether or not that layer has any
meaningful way to respond to the failure. A single low-level failure mode ends up smeared
across the whole call chain instead of staying local to the one module that can actually
do something about it. Good shorthand for "pushing complexity to callers" when the
concrete example is a language feature rather than a home-grown API.

## Dynamic arrays / hash maps (pulling complexity downward, done well)

A hash map absorbs a genuinely hard problem — when to grow the backing array, how to
rehash, how to keep amortized cost low — entirely behind `put`/`get`. Callers never think
about capacity. Contrast with a hypothetical API that made callers pre-size the map or
manually trigger a rehash: that would take an implementation detail that's naturally the
module's problem and hand it to every single caller instead. Useful when someone's
instinct is to make a hard internal choice into a parameter or a caller responsibility —
ask whether it could instead be a hash-map-style "just handle it internally" design.
