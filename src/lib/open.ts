import { inTauri } from "./ipc";
import { toastError } from "../stores/ui";

/**
 * Hand a URL to the system browser.
 *
 * Inside the app a plain `target="_blank"` either does nothing or navigates the
 * webview away from Umbra, so external links go through the opener plugin. Only
 * http(s) is ever passed on: these URLs come from a subscription response, i.e.
 * from a third party, and `javascript:`/`file:` must not be reachable from one.
 */
export function openExternal(url: string): void {
  const clean = url.trim();
  if (!/^https?:\/\//i.test(clean)) return;
  if (!inTauri) {
    window.open(clean, "_blank", "noopener,noreferrer");
    return;
  }
  void import("@tauri-apps/plugin-opener")
    .then((m) => m.openUrl(clean))
    .catch(toastError);
}
