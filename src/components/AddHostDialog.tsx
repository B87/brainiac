import { useId, useState } from "react";
import {
  type AgentHost,
  type AgentHostPreview,
  errorMessage,
  ipc,
} from "../lib/ipc";
import Dialog from "./Dialog";

const STEPS = ["Connect", "Confirm key", "Install", "Build image", "Test"];

/**
 * Add host… (SPEC.md, Remote hosts, Adding a host): the SSH address, then
 * the key the host presents and what Install runs there with sudo. Trust
 * this key and install saves the host and starts install, image build, and
 * test as one job, which goes on after this closes. `existing` is a saved
 * host whose key is confirmed again.
 */
export default function AddHostDialog({
  existing,
  onClose,
  onAdded,
}: {
  existing?: AgentHost;
  onClose: () => void;
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

  return (
    <Dialog
      title={existing ? `Confirm ${existing.name}'s key` : "Add host"}
      width={640}
      onClose={onClose}
      footer={
        <div className="flex w-full flex-wrap items-center gap-2.5">
          <span className="min-w-0 flex-1 text-[12px] text-muted">
            {preview
              ? installs
                ? "Install, Build image, and Test follow one after another. You can close this and follow them in Settings → Agents."
                : "Nothing is installed again: the host keeps its run controller."
              : "A Linux machine with systemd and Docker, reached with your SSH setup. Brainiac keeps no private key."}
          </span>
          {preview ? (
            <>
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
              <button
                type="button"
                className="btn btn-sm btn-primary"
                disabled={busy}
                onClick={() => void trust()}
              >
                {keyChanged
                  ? installs
                    ? "Trust the New Key and Install"
                    : "Trust the New Key"
                  : installs
                    ? "Trust This Key and Install"
                    : "Trust This Key"}
              </button>
            </>
          ) : (
            <>
              <button type="button" className="btn btn-sm" onClick={onClose}>
                Cancel
              </button>
              <button
                type="submit"
                form={`${id}-connect`}
                className="btn btn-sm btn-primary"
                disabled={busy || !user.trim() || !host.trim()}
              >
                {busy ? "Connecting…" : "Show Host Key"}
              </button>
            </>
          )}
        </div>
      }
    >
      <div className="flex flex-col gap-4 px-5 py-4 text-[12.5px]">
        <ol
          aria-label="Steps"
          className="m-0 flex list-none flex-wrap gap-1 p-0 text-[12px]"
        >
          {STEPS.map((label, i) => (
            <li
              key={label}
              aria-current={i === at ? "step" : undefined}
              className="state-pill"
              data-tone={i < at ? "green" : i === at ? "blue" : "dashed"}
            >
              {i + 1} {label}
            </li>
          ))}
        </ol>

        {!preview ? (
          <form
            id={`${id}-connect`}
            className="flex flex-col gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (user.trim() && host.trim()) void look();
            }}
          >
            <div className="grid grid-cols-2 gap-3">
              <Field
                label="Name"
                hint="How Brainiac shows it; the host by default."
              >
                <input
                  className="field"
                  aria-label="Host name"
                  value={name}
                  placeholder={host.trim() || "build-01"}
                  onChange={(e) => setName(e.target.value)}
                />
              </Field>
              <Field label="SSH user" hint="Can run sudo without a password.">
                <input
                  className="field"
                  aria-label="SSH user"
                  value={user}
                  autoCapitalize="off"
                  spellCheck={false}
                  onChange={(e) => setUser(e.target.value)}
                />
              </Field>
              <Field label="Host">
                <input
                  className="field"
                  aria-label="SSH host"
                  value={host}
                  autoCapitalize="off"
                  spellCheck={false}
                  disabled={!!existing}
                  onChange={(e) => setHost(e.target.value)}
                />
              </Field>
              <Field label="Port">
                <input
                  className="field"
                  aria-label="SSH port"
                  value={port}
                  inputMode="numeric"
                  disabled={!!existing}
                  onChange={(e) => setPort(e.target.value)}
                />
              </Field>
            </div>
            <Field
              label="Identity file"
              hint="Optional: an absolute path. Otherwise your SSH agent and configuration choose the key."
            >
              <input
                className="field mono"
                aria-label="Identity file"
                value={identity}
                placeholder="~/.ssh/id_ed25519"
                spellCheck={false}
                onChange={(e) => setIdentity(e.target.value)}
              />
            </Field>
          </form>
        ) : (
          <>
            <p className="m-0 text-fg-2">
              <strong className="font-semibold text-fg">
                {name.trim() || host.trim()}
              </strong>{" "}
              · {user.trim()}@{host.trim()}:{portNumber}
              {identity.trim() && (
                <>
                  {" "}
                  · identity <span className="mono">{identity.trim()}</span>
                </>
              )}
            </p>
            <div className="flex flex-col gap-2">
              <span className="font-semibold">The host presented this key</span>
              <code className="mono selectable break-all rounded-lg border bg-header px-3.5 py-3 text-[13px]">
                {preview.fingerprint}
              </code>
              <span className="text-muted">
                Compare it with what the host's administrator sees, for example
                with{" "}
                <code className="mono">
                  ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
                </code>
                . Brainiac never trusts a key on its own.
              </span>
              {keyChanged && (
                <span role="note" className="text-dirty">
                  This replaces the key Brainiac had saved for this host. SSH
                  will not connect until you trust it.
                </span>
              )}
            </div>
            {installs && (
              <div className="flex flex-col gap-2">
                <span className="font-semibold">
                  Install then runs, with sudo
                </span>
                <ul className="m-0 flex list-disc flex-col gap-1.5 rounded-lg border bg-app py-3 pr-3.5 pl-8 text-fg-2">
                  {preview.actions.map((action) => (
                    <li key={action}>{action}</li>
                  ))}
                </ul>
                <span className="text-muted">
                  The host's administrator can see the repositories and the
                  credential runs get there.
                </span>
              </div>
            )}
          </>
        )}
        {error && (
          <div role="alert" className="text-conflict">
            {error}
          </div>
        )}
      </div>
    </Dialog>
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
    <label className="flex flex-col gap-1">
      <span className="text-[12px] font-medium">{label}</span>
      {children}
      {hint && <span className="text-[11.5px] text-muted">{hint}</span>}
    </label>
  );
}
