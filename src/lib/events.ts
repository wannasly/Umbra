// Single bootstrap: subscribes every backend event channel to the stores and
// loads the initial state. Called exactly once from main.tsx.

import i18n from "../i18n";
import { ipc, onEvent } from "./ipc";
import { useConnection } from "../stores/connection";
import { useLogs } from "../stores/logs";
import { useServers } from "../stores/servers";
import { useSettings } from "../stores/settings";
import { toast, toastError, useUi } from "../stores/ui";

let initialized = false;

export function initApp(): void {
  if (initialized) return;
  initialized = true;

  void onEvent("conn://state", (c) => useConnection.getState().setConn(c));
  void onEvent("traffic://stats", (t) => useConnection.getState().pushTraffic(t));
  void onEvent("core://log", (batch) => useLogs.getState().append(batch));
  void onEvent("ping://result", (r) => useServers.getState().onPingResult(r));
  void onEvent("core://download-progress", (p) =>
    useSettings.getState().setCoreProgress(p),
  );
  void onEvent("core://crashed", (info) => {
    toast({
      kind: "error",
      title: i18n.t("common.coreCrashed", { code: info.code ?? "?" }),
      message: info.willRestart ? i18n.t("common.coreRestarting") : undefined,
    });
  });
  void onEvent("ui://needs-elevation", () =>
    useUi.getState().setElevationOpen(true),
  );
  void onEvent("sub://updated", (u) => {
    toast({
      kind: "success",
      title: i18n.t("subscriptions.updated", {
        added: u.added,
        removed: u.removed,
      }),
    });
    void useServers.getState().refresh().catch(toastError);
  });

  void (async () => {
    await useSettings.getState().load();
    useConnection.getState().setConn(await ipc("get_connection_state"));
    await useServers.getState().refresh();
  })().catch(toastError);
}
