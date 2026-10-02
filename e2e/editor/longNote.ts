/**
 * A long note for scrolling and performance tests: about 3,000 lines of
 * headings, paragraphs, lists, tasks, code, quotes, rules, images from the
 * vault, and a few web images. Generated the same way every time.
 */
const WORDS =
  "retry backoff schedule provider charge attempt review rollout migration index search note task repository branch commit".split(
    " ",
  );

export function longNote(lines = 3000): string {
  let seed = 7;
  const word = () => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return WORDS[seed % WORDS.length];
  };
  const sentence = (n: number) => {
    const text = Array.from({ length: n }, word).join(" ");
    return `${text[0].toUpperCase()}${text.slice(1)}.`;
  };
  const out = ["---", "title: Long note", "tags: [test]", "---", ""];
  for (let k = 1; out.length < lines; k++) {
    out.push(
      `## Section ${k}`,
      "",
      `${sentence(20 + (k % 70))} See [the plan](plan-${k}.md) and [[Note ${k}]] or https://example.com/${k}.`,
      "",
      `- **Point** ${sentence(8)}`,
      `- Uses \`fetch_with_backoff\` ${sentence(6)}`,
      "  - nested *detail* here",
      `- [ ] Task ${k}`,
      `- [x] Done ${k}`,
      "",
    );
    if (k % 3 === 0)
      out.push(`![Diagram ${k}](attachments/diagram-${(k % 6) + 1}.svg)`, "");
    if (k % 25 === 0)
      out.push(`![Remote ${k}](https://example.com/remote-${k}.png)`, "");
    if (k % 4 === 0)
      out.push(
        "```rust",
        "fn main() {",
        '    println!("hello");',
        "}",
        "```",
        "",
      );
    out.push("> A quote with ~~old~~ text", "", "---", "");
  }
  return `${out.join("\n")}\n`;
}
