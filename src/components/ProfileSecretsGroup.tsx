import { useEffect, useState } from "react";
import { errorMessage, ipc, type NamedSecret } from "../lib/ipc";
import { Group, Hint } from "./SettingsPanes";

/**
 * Agents › a profile › Secrets for runs (docs/design/scheduled-runs.md): which
 * of the named secrets from Settings → Secrets this agent's runs get. A run
 * receives each as its variable, from the sealed copy on the run host; the
 * secret has to be shared to that host first. Names only: no value is read.
 */
export default function ProfileSecretsGroup({
  profileId,
}: {
  profileId: string;
}) {
  const [secrets, setSecrets] = useState<NamedSecret[] | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    Promise.all([ipc.listNamedSecrets(), ipc.getProfileSecrets(profileId)])
      .then(([all, ids]) => {
        if (!alive) return;
        setSecrets(all);
        setChosen(ids);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [profileId]);

  const toggle = async (id: string, on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const next = on ? [...chosen, id] : chosen.filter((c) => c !== id);
      setChosen(await ipc.setProfileSecrets(profileId, next));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Group label="Secrets for runs">
      <div className="settings-group flex flex-col">
        {secrets?.length === 0 && (
          <div className="settings-row text-[12.5px] text-muted">
            No secrets yet. Add one in Settings → Secrets, share it with a run
            host, then choose it here.
          </div>
        )}
        {secrets?.map((s) => (
          <label key={s.id} className="settings-row">
            <input
              type="checkbox"
              checked={chosen.includes(s.id)}
              disabled={busy}
              onChange={(e) => void toggle(s.id, e.target.checked)}
            />
            <span className="min-w-0 flex-1 truncate font-medium">
              {s.label}{" "}
              <span className="mono text-[12px] font-normal text-muted">
                {s.env_name}
              </span>
            </span>
          </label>
        ))}
        {error && (
          <div
            className="settings-row text-[12.5px] text-conflict"
            role="alert"
          >
            {error}
          </div>
        )}
      </div>
      <Hint>
        A run on a Linux run host gets each chosen secret as an environment
        variable, from the host's sealed copy, so the secret must be shared to
        that host first; a run on a host without it does not start. A run's
        network is not restricted, so anything it holds can be sent anywhere a
        prompt leads it: choose tokens that can read and nothing more.
      </Hint>
    </Group>
  );
}
