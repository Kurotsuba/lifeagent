import { zhCN } from "../i18n/zh-CN";

/** What a Settings section shows before its data has loaded: the load error if the initial
 * fetch failed, otherwise 加载中… — so a failed load never looks like one still in progress. */
export function LoadState({ error }: { error: string | null }) {
  if (error) {
    return <p className="text-sm text-red-600 dark:text-red-400">{error}</p>;
  }
  return <p className="text-sm text-neutral-500 dark:text-neutral-400">{zhCN.common.loading}</p>;
}
