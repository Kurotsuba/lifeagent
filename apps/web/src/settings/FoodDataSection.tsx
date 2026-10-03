import { useEffect, useRef, useState } from "react";
import {
  cancelFoodImport,
  commitFoodImport,
  deleteFoodSource,
  listFoodSources,
  previewFoodImport,
  renameFoodSource,
  reorderFoodSources,
  setFoodSourceEnabled,
} from "../api/commands";
import {
  commandErrorMessage,
  type ImportPreview,
  type SourceInfo,
} from "../api/types";
import { ConfirmDeleteButton } from "../components/ConfirmDeleteButton";
import { inputClass } from "../components/Field";
import { primaryButtonClass, secondaryButtonClass } from "../components/buttonStyles";
import { describeImportMessage, strings } from "../i18n";
import { LoadState } from "./LoadState";

const t = strings.settings.foodData;

/** "2026-09-26T10:15:00+00:00" -> "2026-09-26". */
function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function kindLabel(source: SourceInfo): string {
  if (source.kind === "user") return t.kindManual;
  if (source.kind === "llm_estimate") return t.kindEstimate;
  return source.origin ? t.kindDataset(formatDate(source.origin.imported_at)) : t.kindDatasetPlain;
}

/** Which file picker invocation is in flight: a brand-new dataset, or a specific dataset's
 * 替换数据 (replace) action. */
interface PendingPick {
  replaceSourceId: string | null;
}

export function FoodDataSection() {
  const [sources, setSources] = useState<SourceInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pick, setPick] = useState<PendingPick | null>(null);
  const [pendingFile, setPendingFile] = useState<File | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  async function refetch() {
    try {
      setSources(await listFoodSources());
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  useEffect(() => {
    void refetch();
  }, []);

  function openFilePicker(replaceSourceId: string | null) {
    setPick({ replaceSourceId });
    fileInputRef.current?.click();
  }

  function handleFileChosen(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = ""; // so choosing the same file again still fires onChange
    if (!file) {
      setPick(null);
      return;
    }
    setPendingFile(file);
  }

  async function withErrorHandling(action: () => Promise<void>) {
    setError(null);
    try {
      await action();
      await refetch();
    } catch (e) {
      setError(commandErrorMessage(e));
    }
  }

  async function handleRename(id: string, currentName: string) {
    const name = window.prompt(t.rename, currentName);
    if (!name || name === currentName) return;
    await withErrorHandling(() => renameFoodSource(id, name));
  }

  async function move(datasets: SourceInfo[], id: string, delta: number) {
    const ids = datasets.map((d) => d.id);
    const i = ids.indexOf(id);
    const j = i + delta;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    await withErrorHandling(() => reorderFoodSources(ids));
  }

  if (!sources) {
    return <LoadState error={error} />;
  }

  const manual = sources.find((s) => s.kind === "user") ?? null;
  const estimate = sources.find((s) => s.kind === "llm_estimate") ?? null;
  const datasets = sources.filter((s) => s.kind === "dataset");

  return (
    <div className="space-y-3">
      <input
        ref={fileInputRef}
        type="file"
        accept=".csv,.json"
        className="hidden"
        onChange={handleFileChosen}
      />
      <div className="flex flex-col items-start gap-2 sm:flex-row sm:items-center sm:justify-between sm:gap-3">
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {t.hint}{" "}
          <span
            className="cursor-help text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300"
            title={t.formatHelp}
          >
            ⓘ
          </span>
        </p>
        <button
          type="button"
          className={`${secondaryButtonClass} shrink-0 !px-3 !py-1.5 text-xs`}
          onClick={() => openFilePicker(null)}
        >
          {t.importButton}
        </button>
      </div>

      {error ? <p className="text-sm text-red-600 dark:text-red-400">{error}</p> : null}

      <div className="space-y-1">
        {manual ? <SourceRow source={manual} label={t.kindManual} locked /> : null}
        {datasets.map((source, i) => (
          <SourceRow
            key={source.id}
            source={source}
            label={kindLabel(source)}
            onToggle={(enabled) =>
              void withErrorHandling(() => setFoodSourceEnabled(source.id, enabled))
            }
            onMoveUp={i > 0 ? () => void move(datasets, source.id, -1) : undefined}
            onMoveDown={i < datasets.length - 1 ? () => void move(datasets, source.id, 1) : undefined}
            onRename={() => void handleRename(source.id, source.name)}
            onReplace={() => openFilePicker(source.id)}
            onDelete={() => withErrorHandling(() => deleteFoodSource(source.id))}
          />
        ))}
        {estimate ? <SourceRow source={estimate} label={t.kindEstimate} locked /> : null}
        {datasets.length === 0 ? (
          <p className="rounded-md border border-dashed border-neutral-300 px-3 py-4 text-center text-xs text-neutral-500 dark:border-neutral-700 dark:text-neutral-400">
            {t.empty}
          </p>
        ) : null}
      </div>

      {pendingFile ? (
        <ImportDialog
          file={pendingFile}
          replaceSourceId={pick?.replaceSourceId ?? null}
          replaceSourceName={
            pick?.replaceSourceId
              ? (sources.find((s) => s.id === pick.replaceSourceId)?.name ?? null)
              : null
          }
          onClose={() => {
            setPendingFile(null);
            setPick(null);
          }}
          onImported={() => {
            setPendingFile(null);
            setPick(null);
            void refetch();
          }}
        />
      ) : null}
    </div>
  );
}

interface SourceRowProps {
  source: SourceInfo;
  label: string;
  locked?: boolean;
  onToggle?: (enabled: boolean) => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  onRename?: () => void;
  onReplace?: () => void;
  onDelete?: () => Promise<void>;
}

function SourceRow({
  source,
  label,
  locked,
  onToggle,
  onMoveUp,
  onMoveDown,
  onRename,
  onReplace,
  onDelete,
}: SourceRowProps) {
  return (
    // The controls wrap under the name when the row is too narrow for both (phones).
    <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 rounded-md border border-neutral-200 px-3 py-2 text-sm dark:border-neutral-700">
      <div className="min-w-0 flex-1 basis-48">
        <p className="truncate text-neutral-900 dark:text-neutral-50">{source.name}</p>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {label} ·{" "}
          {source.dish_count > 0
            ? t.foodAndDishCount(source.food_count, source.dish_count)
            : t.foodCount(source.food_count)}
        </p>
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-1">
        {locked ? (
          <span className="text-sm" title={t.locked} aria-label={t.locked}>
            🔒
          </span>
        ) : (
          <>
            <button
              type="button"
              disabled={!onMoveUp}
              className="rounded px-1 text-neutral-400 hover:text-neutral-700 disabled:opacity-30 dark:hover:text-neutral-200"
              onClick={onMoveUp}
            >
              ↑
            </button>
            <button
              type="button"
              disabled={!onMoveDown}
              className="rounded px-1 text-neutral-400 hover:text-neutral-700 disabled:opacity-30 dark:hover:text-neutral-200"
              onClick={onMoveDown}
            >
              ↓
            </button>
            <label className="relative ml-1 inline-flex cursor-pointer items-center">
              <input
                type="checkbox"
                checked={source.enabled}
                onChange={(e) => onToggle?.(e.target.checked)}
                className="peer sr-only"
              />
              <span className="h-4 w-8 rounded-full bg-neutral-300 transition-colors peer-checked:bg-neutral-900 dark:bg-neutral-600 dark:peer-checked:bg-neutral-100" />
              <span className="absolute left-0.5 top-0.5 h-3 w-3 rounded-full bg-white transition-transform peer-checked:translate-x-4 dark:bg-neutral-900" />
            </label>
            <button
              type="button"
              className="ml-1 whitespace-nowrap text-xs text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200"
              onClick={onRename}
            >
              {t.rename}
            </button>
            <button
              type="button"
              className="whitespace-nowrap text-xs text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200"
              onClick={onReplace}
            >
              {t.replace}
            </button>
            {onDelete ? <ConfirmDeleteButton onConfirm={onDelete} /> : null}
          </>
        )}
      </div>
    </div>
  );
}

interface ImportDialogProps {
  file: File;
  replaceSourceId: string | null;
  replaceSourceName: string | null;
  onClose: () => void;
  onImported: () => void;
}

function ImportDialog({
  file,
  replaceSourceId,
  replaceSourceName,
  onClose,
  onImported,
}: ImportDialogProps) {
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [showIssues, setShowIssues] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const p = await previewFoodImport(file);
        if (cancelled) return;
        setPreview(p);
        setName(p.suggested_name);
      } catch (e) {
        if (!cancelled) setError(commandErrorMessage(e));
      }
    })();
    return () => {
      cancelled = true;
    };
    // `file` is a stable reference for the lifetime of this dialog (a new file re-mounts it via
    // the parent's `key`-less remount on `pendingFile` change).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleCancel() {
    if (preview) {
      try {
        await cancelFoodImport(preview.token);
      } catch {
        // Best effort: an already-committed or expired token is fine to ignore here.
      }
    }
    onClose();
  }

  async function handleImport() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      await commitFoodImport(
        preview.token,
        replaceSourceId ? null : name,
        replaceSourceId,
      );
      onImported();
    } catch (e) {
      setError(commandErrorMessage(e));
      setBusy(false);
    }
  }

  const issues = preview ? [...preview.errors, ...preview.warnings] : [];

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center bg-black/40 p-4">
      <div className="max-h-[85vh] w-full max-w-lg overflow-y-auto rounded-lg bg-white p-4 shadow-xl dark:bg-neutral-800">
        <h3 className="mb-3 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
          {replaceSourceName ? t.replaceTitle(replaceSourceName) : t.importTitle}
        </h3>

        {error ? <p className="mb-2 text-sm text-red-600 dark:text-red-400">{error}</p> : null}

        {!preview ? (
          <LoadState error={null} />
        ) : (
          <div className="space-y-3 text-sm">
            {!replaceSourceId ? (
              <label className="block">
                <span className="mb-1 block text-xs text-neutral-500 dark:text-neutral-400">
                  {t.nameLabel}
                </span>
                <input
                  className={inputClass}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
            ) : null}

            <div className="flex flex-wrap gap-2 text-xs">
              <span className="rounded bg-neutral-100 px-2 py-1 dark:bg-neutral-700">
                {t.countImportableFoods(preview.valid_food_rows)}
              </span>
              {preview.valid_dish_rows > 0 ? (
                <span className="rounded bg-neutral-100 px-2 py-1 dark:bg-neutral-700">
                  {t.countImportableDishes(preview.valid_dish_rows)}
                </span>
              ) : null}
              <span className="rounded bg-neutral-100 px-2 py-1 dark:bg-neutral-700">
                {t.countSkipped(preview.error_count)}
              </span>
              <span className="rounded bg-neutral-100 px-2 py-1 dark:bg-neutral-700">
                {t.countWarnings(preview.warning_count)}
              </span>
            </div>

            {preview.brands.length > 0 ? (
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {t.brandsLabel(preview.brands.join(strings.common.listSeparator))}
              </p>
            ) : null}
            {preview.replaces_estimates > 0 ? (
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {t.replacesEstimates(preview.replaces_estimates)}
              </p>
            ) : null}
            {preview.shadowed_by_manual > 0 ? (
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {t.shadowedByManual(preview.shadowed_by_manual)}
              </p>
            ) : null}
            {preview.dish_shadowed_by_manual > 0 ? (
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {t.dishShadowedByManual(preview.dish_shadowed_by_manual)}
              </p>
            ) : null}

            {issues.length > 0 ? (
              <div>
                <button
                  type="button"
                  className="text-xs text-neutral-500 underline dark:text-neutral-400"
                  onClick={() => setShowIssues((v) => !v)}
                >
                  {showIssues ? t.hideIssues : t.showIssues}
                </button>
                {showIssues ? (
                  <div className="mt-2 max-h-40 overflow-y-auto rounded border border-neutral-200 dark:border-neutral-700">
                    <table className="w-full text-xs">
                      <thead className="bg-neutral-50 dark:bg-neutral-900">
                        <tr>
                          <th className="px-2 py-1 text-left">{t.colLine}</th>
                          <th className="px-2 py-1 text-left">{t.colName}</th>
                          <th className="px-2 py-1 text-left">{t.colProblem}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {issues.map((issue, i) => (
                          <tr key={i} className="border-t border-neutral-100 dark:border-neutral-800">
                            <td className="px-2 py-1">{issue.line || "—"}</td>
                            <td className="px-2 py-1">{issue.name ?? "—"}</td>
                            <td className="px-2 py-1">{describeImportMessage(issue.message)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                ) : null}
              </div>
            ) : null}

            <div>
              <p className="mb-1 text-xs text-neutral-500 dark:text-neutral-400">{t.sampleTitle}</p>
              <div className="max-h-40 overflow-y-auto rounded border border-neutral-200 dark:border-neutral-700">
                <table className="w-full text-xs">
                  <thead className="bg-neutral-50 dark:bg-neutral-900">
                    <tr>
                      <th className="px-2 py-1 text-left">{t.colBasis}</th>
                      <th className="px-2 py-1 text-left">{t.colName}</th>
                      <th className="px-2 py-1 text-right">kcal</th>
                      <th className="px-2 py-1 text-right">{t.colProtein}</th>
                      <th className="px-2 py-1 text-right">{t.colCarbs}</th>
                      <th className="px-2 py-1 text-right">{t.colFat}</th>
                      <th className="px-2 py-1 text-right">{t.colFiber}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {preview.sample.map((row, i) => (
                      <tr key={i} className="border-t border-neutral-100 dark:border-neutral-800">
                        <td className="px-2 py-1 whitespace-nowrap text-neutral-500 dark:text-neutral-400">
                          {row.basis === "serving" ? t.basisDish : t.basisFood}
                        </td>
                        <td className="px-2 py-1">
                          {row.brand ? `${row.brand} ` : ""}
                          {row.name}
                        </td>
                        <td className="px-2 py-1 text-right">{row.kcal}</td>
                        <td className="px-2 py-1 text-right">{row.protein_g}</td>
                        <td className="px-2 py-1 text-right">{row.carbs_g}</td>
                        <td className="px-2 py-1 text-right">{row.fat_g}</td>
                        <td className="px-2 py-1 text-right">{row.fiber_g}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            className={`${secondaryButtonClass} !px-3 !py-1.5 text-xs`}
            disabled={busy}
            onClick={() => void handleCancel()}
          >
            {strings.common.cancel}
          </button>
          <button
            type="button"
            className={`${primaryButtonClass} !px-3 !py-1.5 text-xs`}
            disabled={busy || !preview || preview.valid_rows === 0}
            onClick={() => void handleImport()}
          >
            {t.importAction}
          </button>
        </div>
      </div>
    </div>
  );
}
