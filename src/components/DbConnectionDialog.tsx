import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { ENV_LABEL, ENVIRONMENTS } from "../lib/databases";
import {
  type DbAccess,
  type DbConnection,
  type DbEnvironment,
  type DbKind,
  type DbPassword,
  type DbTls,
  type DockerContainer,
  errorMessage,
  ipc,
  type RunsOn,
  type SaveDbConnectionRequest,
} from "../lib/ipc";
import Dialog from "./Dialog";

type Props = {
  /** The connection being edited; none for New Connection…. */
  connection?: DbConnection;
  onClose: () => void;
  onSaved: (connection: DbConnection) => void;
};

type RunsOnChoice = "none" | "cloud_sql" | "docker";

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
    <div className="flex flex-col gap-1">
      {/* biome-ignore lint/a11y/noLabelWithoutControl: the control is the child. */}
      <label className="flex flex-col gap-1">
        <span className="text-[12px] font-medium text-fg-2">{label}</span>
        {children}
      </label>
      {hint && <span className="text-[11.5px] text-muted">{hint}</span>}
    </div>
  );
}

/** New Connection… and Edit Connection… (SPEC.md, Databases: Connections). */
export default function DbConnectionDialog({
  connection,
  onClose,
  onSaved,
}: Props) {
  const c = connection;
  const [kind, setKind] = useState<DbKind>(c?.kind ?? "postgres");
  const [name, setName] = useState(c?.name ?? "");
  const [environment, setEnvironment] = useState<DbEnvironment>(
    c?.environment ?? "local",
  );
  const [access, setAccess] = useState<DbAccess>(c?.access ?? "read_only");
  const [filePath, setFilePath] = useState(c?.file_path ?? "");
  const [host, setHost] = useState(c?.host ?? "localhost");
  const [port, setPort] = useState(String(c?.port ?? 5432));
  const [database, setDatabase] = useState(c?.database ?? "postgres");
  const [user, setUser] = useState(c?.user ?? "");
  const [password, setPassword] = useState("");
  const [storage, setStorage] = useState<DbPassword>(c?.password ?? "keychain");
  const [tls, setTls] = useState<DbTls>(c?.tls ?? "verify");
  const [caFile, setCaFile] = useState(c?.ca_file ?? "");
  const [timeout, setTimeoutSeconds] = useState(
    String(c?.statement_timeout_seconds ?? 30),
  );
  const [runsOnChoice, setRunsOnChoice] = useState<RunsOnChoice>(
    c?.runs_on?.kind ?? "none",
  );
  const [project, setProject] = useState(
    c?.runs_on?.kind === "cloud_sql" ? c.runs_on.project : "",
  );
  const [instance, setInstance] = useState(
    c?.runs_on?.kind === "cloud_sql" ? c.runs_on.instance : "",
  );
  const [container, setContainer] = useState(
    c?.runs_on?.kind === "docker" ? c.runs_on.container : "",
  );
  const [socket, setSocket] = useState(
    c?.runs_on?.kind === "docker" ? (c.runs_on.socket ?? "") : "",
  );
  const [containers, setContainers] = useState<DockerContainer[] | null>(null);
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [tested, setTested] = useState<string | null>(null);
  const [busy, setBusy] = useState<"test" | "save" | null>(null);

  const runsOn = (): RunsOn | null => {
    if (kind !== "postgres") return null;
    if (runsOnChoice === "cloud_sql")
      return { kind: "cloud_sql", project, instance };
    if (runsOnChoice === "docker")
      return { kind: "docker", container, socket: socket.trim() || null };
    return null;
  };

  const request = (): SaveDbConnectionRequest => ({
    id: c?.id ?? null,
    expected_version: c?.version ?? null,
    name,
    kind,
    environment,
    access,
    file_path: kind === "sqlite" ? filePath : null,
    host: kind === "postgres" ? host : null,
    port: kind === "postgres" ? Number(port) || null : null,
    database: kind === "postgres" ? database : null,
    user: kind === "postgres" ? user : null,
    tls: kind === "postgres" ? tls : null,
    ca_file: kind === "postgres" && tls === "verify" && caFile ? caFile : null,
    password_storage: kind === "postgres" ? storage : "none",
    password:
      kind === "postgres" && storage !== "none" && password ? password : null,
    statement_timeout_seconds: Number(timeout) || 30,
    runs_on: runsOn(),
  });

  const fromUrl = async (text: string) => {
    setUrl(text);
    if (!/^postgres(ql)?:\/\//.test(text.trim())) return;
    try {
      const f = await ipc.parseDbUrl(text);
      if (f.host) setHost(f.host);
      if (f.port) setPort(String(f.port));
      if (f.database) setDatabase(f.database);
      if (f.user) setUser(f.user);
      if (f.password) {
        setPassword(f.password);
        if (storage === "none") setStorage("keychain");
      }
      if (f.tls) setTls(f.tls);
      if (!name && f.database) setName(f.database);
      // The password moved to its field; the URL field keeps no copy of it.
      setUrl("");
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const test = async () => {
    setBusy("test");
    setTested(null);
    setError(null);
    try {
      const result = await ipc.testDbConnection(request());
      setTested(`Connected: ${result.server_version}.`);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    setBusy("save");
    setError(null);
    try {
      onSaved(await ipc.saveDbConnection(request()));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const chooseFile = async (setter: (path: string) => void, title: string) => {
    const path = await open({ multiple: false, directory: false, title });
    if (typeof path === "string") setter(path);
  };

  const findContainers = async () => {
    setError(null);
    try {
      const found = await ipc.listDockerContainers(socket.trim() || null);
      setContainers(found.containers);
      if (!socket) setSocket(found.socket);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <Dialog
      title={c ? "Edit Connection" : "New Connection"}
      width={560}
      onClose={onClose}
      footer={
        <>
          {tested && (
            <span className="mr-auto text-[12px] text-clean">{tested}</span>
          )}
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() => void test()}
          >
            {busy === "test" ? "Testing…" : "Test Connection"}
          </button>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy !== null}
            onClick={() => void save()}
          >
            {c ? "Save" : "Add Connection"}
          </button>
        </>
      }
    >
      <form
        className="flex flex-col gap-3.5"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        {!c && (
          <div className="seg self-start" role="tablist" aria-label="Kind">
            {(["postgres", "sqlite"] as const).map((k) => (
              <button
                key={k}
                type="button"
                role="tab"
                aria-selected={kind === k}
                onClick={() => setKind(k)}
              >
                {k === "postgres" ? "PostgreSQL" : "SQLite"}
              </button>
            ))}
          </div>
        )}
        {kind === "postgres" && !c && (
          <Field
            label="Paste a URL"
            hint="postgres://user:password@host:5432/database fills the fields; the password moves to its own field."
          >
            <input
              className="text-input mono"
              value={url}
              placeholder="postgres://…"
              spellCheck={false}
              onChange={(e) => void fromUrl(e.target.value)}
            />
          </Field>
        )}
        <div className="grid grid-cols-2 gap-3">
          <Field label="Name">
            <input
              className="text-input"
              value={name}
              autoFocus
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
          <Field label="Environment">
            <select
              className="text-input"
              value={environment}
              onChange={(e) => {
                const next = e.target.value as DbEnvironment;
                setEnvironment(next);
                // Production starts read only (SPEC.md, Databases: Safety).
                if (next === "production" && !c) setAccess("read_only");
              }}
            >
              {ENVIRONMENTS.map((env) => (
                <option key={env} value={env}>
                  {ENV_LABEL[env]}
                </option>
              ))}
            </select>
          </Field>
        </div>
        {kind === "sqlite" ? (
          <Field label="Database file">
            <div className="flex gap-2">
              <input
                className="text-input mono flex-1"
                value={filePath}
                spellCheck={false}
                onChange={(e) => setFilePath(e.target.value)}
              />
              <button
                type="button"
                className="btn"
                onClick={() =>
                  void chooseFile(setFilePath, "Choose a SQLite database")
                }
              >
                Choose…
              </button>
            </div>
          </Field>
        ) : (
          <>
            <div className="grid grid-cols-[1fr_96px] gap-3">
              <Field label="Host">
                <input
                  className="text-input mono"
                  value={host}
                  spellCheck={false}
                  onChange={(e) => setHost(e.target.value)}
                />
              </Field>
              <Field label="Port">
                <input
                  className="text-input mono"
                  value={port}
                  inputMode="numeric"
                  onChange={(e) => setPort(e.target.value)}
                />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Field label="Database">
                <input
                  className="text-input mono"
                  value={database}
                  spellCheck={false}
                  onChange={(e) => setDatabase(e.target.value)}
                />
              </Field>
              <Field label="User">
                <input
                  className="text-input mono"
                  value={user}
                  spellCheck={false}
                  onChange={(e) => setUser(e.target.value)}
                />
              </Field>
            </div>
            <Field
              label="Password"
              hint={
                storage === "keychain"
                  ? c?.password === "keychain"
                    ? "Kept in the Keychain. Leave it empty to keep the saved one."
                    : "Kept in the Keychain, never in Brainiac's files."
                  : storage === "ask"
                    ? "Asked for once each time Brainiac runs."
                    : "For servers that trust local users."
              }
            >
              <div className="flex gap-2">
                <input
                  className="text-input flex-1"
                  type="password"
                  value={password}
                  disabled={storage === "none"}
                  autoComplete="off"
                  onChange={(e) => setPassword(e.target.value)}
                />
                <select
                  className="text-input"
                  aria-label="Where the password is kept"
                  value={storage}
                  onChange={(e) => setStorage(e.target.value as DbPassword)}
                >
                  <option value="keychain">In the Keychain</option>
                  <option value="ask">Ask each run</option>
                  <option value="none">No password</option>
                </select>
              </div>
            </Field>
            <Field
              label="TLS"
              hint={
                tls === "require"
                  ? "Encrypted, but any certificate is accepted: a server pretending to be this one would not be noticed."
                  : tls === "off"
                    ? "Plain text: only on a network you trust, such as this Mac."
                    : "Verifies the certificate and host name with the Mac's trust store and the CA file."
              }
            >
              <select
                className="text-input"
                value={tls}
                onChange={(e) => setTls(e.target.value as DbTls)}
              >
                <option value="verify">Verify</option>
                <option value="require">Require without verifying</option>
                <option value="off">Off</option>
              </select>
            </Field>
            {tls === "verify" && (
              <Field
                label="CA file (optional)"
                hint="For providers whose certificates the Mac does not trust, such as Amazon RDS."
              >
                <div className="flex gap-2">
                  <input
                    className="text-input mono flex-1"
                    value={caFile}
                    spellCheck={false}
                    onChange={(e) => setCaFile(e.target.value)}
                  />
                  <button
                    type="button"
                    className="btn"
                    onClick={() =>
                      void chooseFile(setCaFile, "Choose a CA certificate")
                    }
                  >
                    Choose…
                  </button>
                </div>
              </Field>
            )}
          </>
        )}
        <div className="grid grid-cols-2 gap-3">
          <Field
            label="Access"
            hint={
              access === "read_only"
                ? "Statements cannot change data."
                : environment === "production"
                  ? "Each tab still starts read only; read and write is turned on per tab."
                  : "Tabs can write. A read-only database role is the real guarantee."
            }
          >
            <select
              className="text-input"
              value={access}
              onChange={(e) => setAccess(e.target.value as DbAccess)}
            >
              <option value="read_only">Read only</option>
              <option value="read_write">Read and write</option>
            </select>
          </Field>
          <Field label="Time limit" hint="Seconds a statement may run.">
            <input
              className="text-input"
              value={timeout}
              inputMode="numeric"
              onChange={(e) => setTimeoutSeconds(e.target.value)}
            />
          </Field>
        </div>
        {kind === "postgres" && (
          <Field
            label="Runs on"
            hint="Where the server runs, for Health's memory, CPU, and disk."
          >
            <select
              className="text-input"
              value={runsOnChoice}
              onChange={(e) => setRunsOnChoice(e.target.value as RunsOnChoice)}
            >
              <option value="none">Not set</option>
              <option value="cloud_sql">Google Cloud SQL</option>
              <option value="docker">Docker container on this Mac</option>
            </select>
          </Field>
        )}
        {kind === "postgres" && runsOnChoice === "cloud_sql" && (
          <div className="grid grid-cols-2 gap-3">
            <Field label="Project">
              <input
                className="text-input mono"
                value={project}
                spellCheck={false}
                onChange={(e) => setProject(e.target.value)}
              />
            </Field>
            <Field
              label="Instance"
              hint="Read with gcloud's credentials (gcloud auth application-default login)."
            >
              <input
                className="text-input mono"
                value={instance}
                spellCheck={false}
                onChange={(e) => setInstance(e.target.value)}
              />
            </Field>
          </div>
        )}
        {kind === "postgres" && runsOnChoice === "docker" && (
          <Field label="Container">
            <div className="flex gap-2">
              {containers ? (
                <select
                  className="text-input flex-1"
                  value={container}
                  onChange={(e) => setContainer(e.target.value)}
                >
                  <option value="">Choose a container</option>
                  {containers.map((d) => (
                    <option key={d.id} value={d.name}>
                      {d.name} · {d.image}
                      {d.ports.length ? ` · ${d.ports.join(", ")}` : ""}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  className="text-input mono flex-1"
                  value={container}
                  placeholder="Name or ID"
                  spellCheck={false}
                  onChange={(e) => setContainer(e.target.value)}
                />
              )}
              <button
                type="button"
                className="btn"
                onClick={() => void findContainers()}
              >
                List Containers
              </button>
            </div>
          </Field>
        )}
        {error && (
          <div
            role="alert"
            className="selectable rounded-md border border-red-300 bg-red-50 px-3 py-2 text-[12.5px] text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100"
          >
            {error}
          </div>
        )}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
