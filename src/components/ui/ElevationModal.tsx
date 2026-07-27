import { useTranslation } from "react-i18next";
import { ShieldAlert } from "lucide-react";
import { ipc } from "../../lib/ipc";
import { useUi, toastError } from "../../stores/ui";
import { Modal } from "./Modal";
import { Button } from "./Button";

/**
 * Mounted once at the app root: TUN can be requested from the dashboard, the
 * tray or the `--resume-tun` startup path, and all three end up here.
 */
export function ElevationModal() {
  const { t } = useTranslation();
  const open = useUi((s) => s.elevationOpen);
  const setOpen = useUi((s) => s.setElevationOpen);

  const restart = () => {
    // On success the backend exits this process, so only failures come back —
    // a declined UAC prompt among them.
    void ipc("relaunch_elevated").catch((e) => {
      setOpen(false);
      toastError(e);
    });
  };

  return (
    <Modal
      open={open}
      onClose={() => setOpen(false)}
      title={t("common.elevation.title")}
    >
      <div className="flex gap-3">
        <ShieldAlert size={18} className="mt-0.5 shrink-0 text-warn" />
        <p className="text-[13px] text-text-dim">{t("common.elevation.body")}</p>
      </div>
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={() => setOpen(false)}>
          {t("common.cancel")}
        </Button>
        <Button onClick={restart}>{t("common.elevation.restart")}</Button>
      </div>
    </Modal>
  );
}
