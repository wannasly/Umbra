import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import en from "./locales/en.json";
import ru from "./locales/ru.json";

// LanguageDetector is only the first-boot fallback; once settings are loaded,
// the persisted settings.language takes priority (settings store calls
// i18n.changeLanguage).
void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      ru: { translation: ru },
    },
    fallbackLng: "en",
    supportedLngs: ["en", "ru"],
    interpolation: { escapeValue: false },
    detection: { order: ["navigator"], caches: [] },
  });

// Keep <html lang> pointing at the language actually on screen. index.html can
// only carry a static boot value, so without this the document stays tagged as
// one language while rendering the other — which is what screen readers,
// hyphenation and font fallback go by.
const syncDocumentLang = (lng: string) => {
  if (typeof document !== "undefined") {
    document.documentElement.lang = lng;
  }
};
syncDocumentLang(i18n.language);
i18n.on("languageChanged", syncDocumentLang);

export default i18n;
