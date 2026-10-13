import { useEffect, useState } from "react";
import {
  getAppState,
  getFollowUpSettings,
  getPlugins,
  saveFollowUpSettings,
  setActivityTabVisible,
  setPluginEnabled,
} from "../api/commands";
import {
  commandErrorMessage,
  type FollowUpSettings,
  type FollowUpTimes,
  type PluginInfo,
} from "../api/types";
import { inputClass } from "../components/Field";
import { strings } from "../i18n";
import { LoadState } from "./LoadState";

const t = strings.settings.features;

/** On/off switches for the optional plugins, and whether the Activity tab shows (it records
 * either way). A switch saves immediately; `onChanged` lets the shell show or hide the page. */
export function FeaturesSection({ onChanged }: { onChanged: () => void }) {
  const [plugins, setPlugins] = useState<PluginInfo[] | null>(null);
  const [activityTab, setActivityTab] = useState<boolean | null>(null);
  const [followUps, setFollowUps] = useState<FollowUpSettings | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([getPlugins(), getAppState(), getFollowUpSettings()])
      .then(([p, state, f]) => {
        if (cancelled) return;
        setPlugins(p);
        setActivityTab(state.activity_tab_visible);
        setFollowUps(f);
      })
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

  async function toggleActivityTab(visible: boolean) {
    setBusy("activity");
    setError(null);
    try {
      await setActivityTabVisible(visible);
      setActivityTab(visible);
      onChanged();
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  // Saves at once, like the switches: a time input only fires `change` with a complete value.
  async function saveFollowUps(next: FollowUpSettings) {
    setBusy("follow_ups");
    setError(null);
    try {
      await saveFollowUpSettings(next);
      setFollowUps(next);
    } catch (e) {
      setError(commandErrorMessage(e));
    } finally {
      setBusy(null);
    }
  }

  if (!plugins || activityTab === null || !followUps) return <LoadState error={error} />;

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
      <label className="flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          className="mt-1"
          checked={activityTab}
          disabled={busy !== null}
          onChange={(e) => void toggleActivityTab(e.target.checked)}
        />
        <span>
          <span className="font-medium">{t.activityTab}</span>
          <span className="mt-0.5 block text-xs text-neutral-500 dark:text-neutral-400">
            {t.activityTabHint}
          </span>
        </span>
      </label>
      <div className="text-sm">
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            className="mt-1"
            checked={followUps.enabled}
            disabled={busy !== null}
            onChange={(e) => void saveFollowUps({ ...followUps, enabled: e.target.checked })}
          />
          <span>
            <span className="font-medium">{t.followUps}</span>
            <span className="mt-0.5 block text-xs text-neutral-500 dark:text-neutral-400">
              {t.followUpsHint}
            </span>
          </span>
        </label>
        {followUps.enabled ? (
          <fieldset className="mt-2 ml-6">
            <legend className="text-xs text-neutral-500 dark:text-neutral-400">
              {t.followUpTimes}
            </legend>
            <div className="mt-1 grid grid-cols-2 gap-2 sm:grid-cols-4">
              {(Object.keys(t.followUpTimeLabels) as (keyof FollowUpTimes)[]).map((key) => (
                <label key={key} className="text-xs">
                  {t.followUpTimeLabels[key]}
                  <input
                    type="time"
                    className={inputClass}
                    value={followUps.times[key]}
                    disabled={busy !== null}
                    onChange={(e) => {
                      if (e.target.value === "") return;
                      void saveFollowUps({
                        ...followUps,
                        times: { ...followUps.times, [key]: e.target.value },
                      });
                    }}
                  />
                </label>
              ))}
            </div>
          </fieldset>
        ) : null}
      </div>
      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}
    </div>
  );
}
