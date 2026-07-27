import { create } from "zustand";
import { ipc, type DownloadProgress, type Settings } from "../lib/ipc";
import i18n from "../i18n";
import { toastError } from "./ui";

interface SettingsStore {
  settings: Settings | null;
  /** live core download progress (core://download-progress), null when idle */
  coreProgress: DownloadProgress | null;
  load: () => Promise<void>;
  patch: (patch: Partial<Settings>) => Promise<void>;
  /** local-only mutation (e.g. selection persisted by another command) */
  setLocal: (patch: Partial<Settings>) => void;
  setCoreProgress: (p: DownloadProgress | null) => void;
}

function applySideEffects(s: Settings): void {
  document.documentElement.dataset.accent = s.accent;
  if (i18n.language !== s.language) void i18n.changeLanguage(s.language);
}

export const useSettings = create<SettingsStore>((set, get) => ({
  settings: null,
  coreProgress: null,
  load: async () => {
    const settings = await ipc("get_settings");
    set({ settings });
    applySideEffects(settings);
  },
  patch: async (patch) => {
    const current = get().settings;
    if (current) {
      const optimistic = { ...current, ...patch };
      set({ settings: optimistic });
      applySideEffects(optimistic);
    }
    try {
      const settings = await ipc("set_settings", { patch });
      set({ settings });
      applySideEffects(settings);
    } catch (e) {
      // roll the optimistic update back so the UI reflects reality
      if (current) {
        set({ settings: current });
        applySideEffects(current);
      }
      toastError(e);
    }
  },
  setLocal: (patch) =>
    set((s) => (s.settings ? { settings: { ...s.settings, ...patch } } : s)),
  setCoreProgress: (coreProgress) => set({ coreProgress }),
}));
