import { useId, useState } from "react";
import {
  type AgentHost,
  type AgentHostPreview,
  errorMessage,
  ipc,
} from "../lib/ipc";
import { PageTitle } from "./SettingsPanes";

const STEPS = ["Address", "Trust its key", "Install the run controller"];

/** What Install checks on the host, in the order it does. */
const CHECKS: { title: string; detail: string }[] = [
  {
    title: "SSH works",
    detail:
      "Logged in as the user you give, who can run sudo without a password.",
  },
  {
    title: "Docker is running",
    detail: "Runs start in containers on its engine.",
  },
  {
    title: "Stays on after you log out",
    detail:
      "The run controller is a systemd service under its own user, so routines fire with this Mac asleep.",
  },
  {
    title: "systemd 256 or newer can seal secrets",
    detail:
      "An older host still runs routines, but none that needs a secret: it keeps no copy.",
  },
];

/**
 * Run hosts › Add a host (SPEC.md, Remote hosts, Adding a host): a page in
 * steps. Address asks for the SSH address; Trust its key shows the key the
 * host presented, to compare from a session already trusted; It matches
 * saves the host and starts install, image build, and test as one job, which
 * the host's page then follows. `existing` is a saved host whose key is
 * confirmed again.
 */
export default function AddHostPage({
  existing,
  onBack,
  onAdded,
}: {
  existing?: AgentHost;
  onBack: () => void;
  /** The saved host, once its key is trusted and its job (if any) started. */
  onAdded: (hostId: string) => void;
}) {
  const id = useId();
  const [name, setName] = useState(existing?.name ?? "");
  const [user, setUser] = useState(existing?.ssh_user ?? "");
  const [host, setHost] = useState(existing?.ssh_host ?? "");
  const [port, setPort] = useState(String(existing?.ssh_port ?? 22));
  const [identity, setIdentity] = useState(existing?.identity_path ?? "");
  const [preview, setPreview] = useState<AgentHostPreview | null>(null);
  const [changed, setChanged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const portNumber = Number(port) || 22;
  const at = preview ? 1 : 0;
  // A saved host's key that differs from the one shown must be approved as a change.
  const keyChanged =
    changed ||
    (preview != null &&
      existing?.fingerprint != null &&
      existing.fingerprint !== preview.fingerprint);

  const look = async () => {
    setBusy(true);
    setError(null);
    setChanged(false);
    try {
      setPreview(await ipc.previewAgentHost(host.trim(), portNumber));
    } catch (e) {
      setPreview(null);
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const trust = async () => {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.approveAgentHost({
        name: name.trim(),
        user: user.trim(),
        host: host.trim(),
        port: portNumber,
        identity_path: identity.trim() || null,
        fingerprint: preview.fingerprint,
        accept_changed_key: keyChanged,
      });
      if (!saved.installed) await ipc.startAgentHostJob(saved.id, "setup");
      onAdded(saved.id);
    } catch (e) {
      const message = errorMessage(e);
      if (!keyChanged && message.includes("Approve the new fingerprint"))
        setChanged(true);
      setError(message);
    } finally {
      setBusy(false);
    }
  };

  const installs = !existing?.installed;
  const title = existing ? `Confirm ${existing.name}'s key` : "Add a host";
  const shownName = name.trim() || host.trim() || "The host";

  const addressFields = (
    <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,1.6fr)_72px] gap-3">
      <Field label="Name" hint="How Brainiac shows it; the host by default.">
        <input
          className="field w-full"
          aria-label="Host name"
          value={name}
          placeholder={host.trim() || "build-01"}
          disabled={!!preview}
          onChange={(e) => setName(e.target.value)}
        />
      </Field>
      <Field label="SSH user" hint="Can run sudo without a password.">
        <input
          className="field w-full"
          aria-label="SSH user"
          value={user}
          autoCapitalize="off"
          spellCheck={false}
          disabled={!!preview}
          onChange={(e) => setUser(e.target.value)}
        />
      </Field>
      <Field label="Host">
        <input
          className="field w-full"
          aria-label="SSH host"
          value={host}
          autoCapitalize="off"
          spellCheck={false}
          disabled={!!existing || !!preview}
          onChange={(e) => setHost(e.target.value)}
        />
      </Field>
      <Field label="Port">
        <input
          className="field w-full"
          aria-label="SSH port"
          value={port}
          inputMode="numeric"
          disabled={!!existing || !!preview}
          onChange={(e) => setPort(e.target.value)}
        />
      </Field>
    </div>
  );

  return (
    <section aria-label={title} className="flex flex-col gap-4 text-[12.5px]">
      <PageTitle title={title} crumb={{ label: "Run hosts", onBack }}>
        {existing
          ? "The key this host presents now, to compare with what it shows for itself."
          : "A Linux machine that stays on, so routines run with your Mac asleep."}
      </PageTitle>

      <ol
        aria-label="Steps"
        className="m-0 flex list-none flex-wrap items-center gap-2 p-0 text-[12px]"
      >
        {STEPS.map((label, i) => (
          <li key={label} className="flex items-center gap-2">
            {i > 0 && (
              <span aria-hidden="true" className="text-faint">
                —
              </span>
            )}
            <span
              aria-current={i === at ? "step" : undefined}
              className="state-pill"
              data-tone={i < at ? "green" : i === at ? "blue" : "dashed"}
            >
              {i < at ? "✓ " : `${i + 1} `}
              {i === 2 && !installs ? "Keep its run controller" : label}
            </span>
          </li>
        ))}
        <li className="flex items-center gap-2 text-muted">
          <span aria-hidden="true" className="text-faint">
            →
          </span>
          The host's page
        </li>
      </ol>

      <div className="flex flex-wrap items-start gap-5">
        <div className="flex min-w-[420px] flex-[1.2] flex-col gap-3.5">
          {preview && (
            <section
              aria-label="The host's key"
              className="flex flex-col gap-2.5 rounded-[10px] border bg-app px-4 py-3.5"
            >
              <h3 className="m-0 text-[13.5px] font-semibold">
                {shownName} answered at {host.trim()}:{portNumber}
              </h3>
              <p className="m-0 text-fg-2">
                Brainiac connects only to a key you have trusted. Compare this
                with what the host shows for itself, from a session you already
                trust (your provider's console, or a terminal you are already
                in):
              </p>
              <code className="mono selectable break-all rounded-lg border bg-header px-3.5 py-3 text-[13px]">
                {preview.fingerprint}
              </code>
              <span className="text-muted">
                On the host:{" "}
                <code className="mono text-fg-2">
                  ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
                </code>
              </span>
              {keyChanged && (
                <span role="note" className="text-dirty">
                  This replaces the key Brainiac had saved for this host. SSH
                  will not connect until you trust it.
                </span>
              )}
              <div className="flex flex-wrap items-center gap-2.5 pt-1">
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  disabled={busy}
                  onClick={() => void trust()}
                >
                  {keyChanged
                    ? installs
                      ? "It Matches: Trust the New Key and Install"
                      : "It Matches: Trust the New Key"
                    : installs
                      ? "It Matches: Trust This Key and Install"
                      : "It Matches: Trust This Key"}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={() => {
                    setPreview(null);
                    setError(null);
                  }}
                >
                  Back
                </button>
                <span className="text-[12px] text-muted">
                  {installs
                    ? "Install, image build, and test follow as one job, on the host's page."
                    : "Nothing is installed again: the host keeps its run controller."}
                </span>
              </div>
            </section>
          )}

          <form
            id={`${id}-connect`}
            aria-label="Address"
            className="flex flex-col gap-3 rounded-[10px] border bg-app px-4 py-3.5"
            onSubmit={(e) => {
              e.preventDefault();
              if (!preview && user.trim() && host.trim()) void look();
            }}
          >
            <div className="flex items-center justify-between">
              <h3 className="m-0 text-[13.5px] font-semibold">Address</h3>
              {preview && (
                <span className="text-[12px] text-clean">✓ done</span>
              )}
            </div>
            {addressFields}
            <Field
              label="Identity file"
              hint="Optional: an absolute path. Otherwise your SSH agent and configuration choose the key."
            >
              <input
                className="field mono w-full"
                aria-label="Identity file"
                value={identity}
                placeholder="~/.ssh/id_ed25519"
                spellCheck={false}
                disabled={!!preview}
                onChange={(e) => setIdentity(e.target.value)}
              />
            </Field>
            {!preview && (
              <div className="flex flex-wrap items-center gap-2.5 pt-1">
                <button
                  type="submit"
                  className="btn btn-sm btn-primary"
                  disabled={busy || !user.trim() || !host.trim()}
                >
                  {busy ? "Connecting…" : "Show Host Key"}
                </button>
                <button type="button" className="btn btn-sm" onClick={onBack}>
                  Cancel
                </button>
                <span className="text-[12px] text-muted">
                  Reached with your SSH setup. Brainiac keeps no private key.
                </span>
              </div>
            )}
          </form>

          {error && (
            <div role="alert" className="text-conflict">
              {error}
            </div>
          )}
        </div>

        <aside
          aria-label="Checked when it installs"
          className="run-card min-w-[300px] flex-1 gap-1"
          data-tone="blue"
        >
          <h3 className="m-0 mb-1 text-[13.5px] font-semibold">
            Checked when it installs
          </h3>
          <ul className="m-0 flex list-none flex-col p-0">
            {CHECKS.map((c) => (
              <li
                key={c.title}
                className="flex items-start gap-2.5 border-line-soft border-b py-2 last:border-b-0"
              >
                <span
                  aria-hidden="true"
                  className="mt-[7px] h-2 w-2 shrink-0 rounded-full bg-faint"
                />
                <span className="flex flex-col">
                  <span className="font-semibold">{c.title}</span>
                  <span className="text-[12px] text-fg-2">{c.detail}</span>
                </span>
              </li>
            ))}
          </ul>
          {preview && installs && (
            <div className="mt-2 flex flex-col gap-1.5">
              <span className="font-semibold">
                Install then runs, with sudo
              </span>
              <ul className="m-0 flex list-disc flex-col gap-1 pl-5 text-[12px] text-fg-2">
                {preview.actions.map((action) => (
                  <li key={action}>{action}</li>
                ))}
              </ul>
            </div>
          )}
          <span className="mt-2 text-[12px] text-muted">
            Install adds the run controller as a service under its own user,
            brainiac, in the docker group, and nothing else. The host's
            administrator can see the repository and the credential runs get
            there. Remove host… on its page undoes all of it.
          </span>
        </aside>
      </div>
    </section>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: the input is the child
    <label className="flex min-w-0 flex-col gap-1">
      <span className="text-[12px] font-medium">{label}</span>
      {children}
      {hint && <span className="text-[11.5px] text-muted">{hint}</span>}
    </label>
  );
}
