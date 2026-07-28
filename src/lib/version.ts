import packageJson from "../../package.json";

export const APP_VERSION = packageJson.version;

export const APP_VERSION_SHORT = APP_VERSION
  .split(".")
  .slice(0, 2)
  .join(".");
