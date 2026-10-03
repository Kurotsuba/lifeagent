import { useEffect, useState } from "react";
import { getPlugins, setPluginEnabled } from "../api/commands";
import { commandErrorMessage, type PluginInfo } from "../api/types";
import { strings } from "../i18n";
import { LoadState } from "./LoadState";

const t = strings.settings.features;

/** On/off switches for the optional plugins. A switch saves immediately; `onChanged` lets the
 * shell show or hide the plugin's page. */
export function FeaturesSection({ onChanged }: { onChanged: () => void }) {
  const [plugins, setPlugins] = useState<PluginInfo[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getPlugins()
      .then((p) => !cancelled && setPlugins(p))
      .catch((e: unknown) => !cancelled && setError(commandErrorMessage(e)));
    return () => {
      cancelled = true;
    };
  }, []);

  async function toggle(id: string, enabled: boolean) {
    setBusy(id);
    setError(null);
    try {
      await setPluginEnabled(id, enabled);
      setPlugins(await getPlugins());
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  if (!plugins) return <LoadState error={error} />;

  const optional = plugins.filter((p) => p.optional);

  return (
    <div className="space-y-3">
      <p className="text-xs text-neutral-500 dark:text-neutral-400">{t.hint}</p>
      {optional.map((plugin) => (
        <label key={plugin.id} className="flex items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-1"
            checked={plugin.enabled}
            disabled={busy !== null}
            onChange={(e) => void toggle(plugin.id, e.target.checked)}
          />
          <span>
            <span className="font-medium">{t.names[plugin.id] ?? plugin.name}</span>
            {t.descriptions[plugin.id] ? (
              <span className="mt-0.5 block text-xs text-neutral-500 dark:text-neutral-400">
                {t.descriptions[plugin.id]}
              </span>
            ) : null}
          </span>
        </label>
      ))}
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
    </div>
  );
}
