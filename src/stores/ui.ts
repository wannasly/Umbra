import { create } from "zustand";
import i18n from "../i18n";
import { isIpcError, type IpcError } from "../lib/ipc";

export type Page =
  | "dashboard"
  | "servers"
  | "subscriptions"
  | "routing"
  | "logs"
  | "settings";

export type ToastKind = "success" | "error" | "info";

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface ToastItem {
  id: number;
  kind: ToastKind;
  title: string;
  message?: string;
  action?: ToastAction;
}

interface UiState {
  page: Page;
  setPage: (page: Page) => void;
  toasts: ToastItem[];
  toast: (item: Omit<ToastItem, "id">) => void;
  dismissToast: (id: number) => void;
  /** "restart as admin" prompt, opened by any TUN attempt that lacks rights */
  elevationOpen: boolean;
  setElevationOpen: (open: boolean) => void;
}

let nextToastId = 0;

export const useUi = create<UiState>((set, get) => ({
  page: "dashboard",
  setPage: (page) => set({ page }),
  toasts: [],
  toast: (item) => {
    const id = ++nextToastId;
    set((s) => ({ toasts: [...s.toasts, { ...item, id }].slice(-3) }));
    setTimeout(() => get().dismissToast(id), 4000);
  },
  dismissToast: (id) =>
    set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),
  elevationOpen: false,
  setElevationOpen: (elevationOpen) => set({ elevationOpen }),
}));

/** Imperative helper for non-component code (stores, event bootstrap). */
export const toast = (item: Omit<ToastItem, "id">): void =>
  useUi.getState().toast(item);

/**
 * Backend error text is untranslated by design (it carries parser/network
 * detail), but these two are ordinary user situations with a concrete next
 * step, so they get a localized message instead.
 */
const TRANSLATED_ERRORS: Partial<Record<IpcError["code"], string>> = {
  HWID_REQUIRED: "common.errors.hwidRequired",
  DEVICE_LIMIT: "common.errors.deviceLimit",
};

/** Standard .catch handler for fire-and-forget IPC calls: surfaces the failure. */
export const toastError = (e: unknown): void => {
  const key = isIpcError(e) ? TRANSLATED_ERRORS[e.code] : undefined;
  toast({
    kind: "error",
    title: i18n.t("common.error"),
    message: key
      ? i18n.t(key)
      : isIpcError(e)
        ? e.message
        : e instanceof Error
          ? e.message
          : undefined,
  });
};

/** Whether a rejection is the backend asking for admin rights. */
export const isElevationError = (e: unknown): boolean =>
  isIpcError(e) && e.code === "NEEDS_ELEVATION";

/**
 * .catch handler for anything that can hit TUN: NEEDS_ELEVATION is a flow the
 * user can act on (restart as admin), not a toast to read and dismiss.
 */
export const handleIpcError = (e: unknown): void => {
  if (isElevationError(e)) {
    useUi.getState().setElevationOpen(true);
    return;
  }
  toastError(e);
};
