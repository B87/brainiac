import type { Tone } from "../lib/routines";

const TONE_CLASS: Record<Tone, string> = {
  ok: "text-clean",
  warn: "text-dirty",
  bad: "text-conflict",
  work: "text-link",
  idle: "text-muted",
};

/** A state in words, with a dot in its colour (the colour is never alone). */
export default function StatePill({
  tone,
  children,
}: {
  tone: Tone;
  children: string;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 whitespace-nowrap ${TONE_CLASS[tone]}`}
    >
      <span
        aria-hidden="true"
        className="inline-block h-[7px] w-[7px] rounded-full bg-current"
      />
      {children}
    </span>
  );
}
