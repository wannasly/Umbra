// Country detection for server names.
//
// Panels name their nodes for humans, not for parsers: "🇵🇱 Poland #1",
// "NL-3 · reality", "Германия (Франкфурт)". We want the country as its own
// element instead of an emoji glued to the front of the label — but a wrong
// flag is worse than no flag, so every rule here is one a human would agree
// with at a glance, and anything else returns null.

const RI_FIRST = 0x1f1e6; // 🇦
const RI_LAST = 0x1f1ff; // 🇿
const A = "A".charCodeAt(0);

export interface FlagInfo {
  /** regional-indicator pair, e.g. "🇵🇱" */
  emoji: string;
  /** ISO 3166-1 alpha-2, e.g. "PL" — shown where emoji flags do not render */
  code: string;
}

export interface ServerName {
  /** the name with a detected leading flag emoji removed */
  label: string;
  /** null whenever the country could not be read with confidence */
  flag: FlagInfo | null;
}

const isRegionalIndicator = (cp: number | undefined): cp is number =>
  cp !== undefined && cp >= RI_FIRST && cp <= RI_LAST;

/** "PL" -> "🇵🇱" */
export function flagEmoji(code: string): string {
  return [...code.toUpperCase()]
    .map((c) => String.fromCodePoint(RI_FIRST + (c.charCodeAt(0) - A)))
    .join("");
}

/**
 * Countries these panels actually sell, keyed by ISO code. Aliases of 5+ chars
 * are matched as substrings, which makes the Russian ones survive declension
 * ("Германия" / "Германии" both contain "герман"); shorter ones must match a
 * whole word so "USA" cannot fire inside another word.
 */
const ALIASES: Record<string, string[]> = {
  NL: ["netherlands", "holland", "нидерланд", "голланд", "амстердам", "amsterdam"],
  DE: ["germany", "герман", "франкфурт", "frankfurt"],
  PL: ["poland", "польш", "варшав", "warsaw"],
  FI: ["finland", "финлянд", "хельсинки", "helsinki"],
  FR: ["france", "франц", "париж", "paris"],
  GB: ["united kingdom", "britain", "england", "великобритан", "англи", "лондон", "london"],
  US: ["united states", "usa", "америк", "сша"],
  CA: ["canada", "канад"],
  JP: ["japan", "япони", "токио", "tokyo"],
  KR: ["south korea", "korea", "коре"],
  CN: ["china", "китай", "китае"],
  HK: ["hong kong", "гонконг"],
  TW: ["taiwan", "тайван"],
  SG: ["singapore", "сингапур"],
  IN: ["india", "инди"],
  TR: ["turkey", "turkiye", "турци", "стамбул", "istanbul"],
  AE: ["emirates", "эмират", "дубай", "dubai"],
  IL: ["israel", "израил"],
  SE: ["sweden", "швеци", "стокгольм", "stockholm"],
  NO: ["norway", "норвег"],
  DK: ["denmark", "дани"],
  IS: ["iceland", "исланди"],
  IE: ["ireland", "ирланди"],
  ES: ["spain", "испани"],
  PT: ["portugal", "португал"],
  IT: ["italy", "итали", "милан", "milan"],
  CH: ["switzerland", "швейцар", "цюрих", "zurich"],
  AT: ["austria", "австри", "вена "],
  BE: ["belgium", "бельги"],
  LU: ["luxembourg", "люксембург"],
  CZ: ["czech", "чехи", "прага", "prague"],
  SK: ["slovakia", "словаки"],
  SI: ["slovenia", "словени"],
  HU: ["hungary", "венгри"],
  RO: ["romania", "румын"],
  BG: ["bulgaria", "болгар"],
  RS: ["serbia", "серби"],
  HR: ["croatia", "хорват"],
  GR: ["greece", "греци"],
  CY: ["cyprus", "кипр"],
  MT: ["malta", "мальт"],
  EE: ["estonia", "эстони", "таллин"],
  LV: ["latvia", "латви", "рига"],
  LT: ["lithuania", "литв"],
  UA: ["ukraine", "украин"],
  MD: ["moldova", "молдов"],
  BY: ["belarus", "беларус", "белорус"],
  RU: ["russia", "росси", "москв", "moscow", "питер"],
  KZ: ["kazakhstan", "казахстан"],
  UZ: ["uzbekistan", "узбекистан"],
  KG: ["kyrgyz", "киргиз", "кыргыз"],
  AM: ["armenia", "армени", "ереван"],
  GE: ["georgia", "грузи", "тбилиси"],
  AZ: ["azerbaijan", "азербайджан"],
  AU: ["australia", "австрали", "сидней", "sydney"],
  BR: ["brazil", "бразил"],
  AR: ["argentina", "аргентин"],
  MX: ["mexico", "мексик"],
  ZA: ["south africa", "юар "],
  EG: ["egypt", "египет"],
  IR: ["iran", "иран"],
  VN: ["vietnam", "вьетнам"],
  TH: ["thailand", "таиланд"],
  ID: ["indonesia", "индонези"],
  MY: ["malaysia", "малайзи"],
  PH: ["philippines", "филиппин"],
};

/**
 * ISO codes we accept as a bare token ("NL-1", "[DE] 2"). Deliberately missing
 * the ones that collide with ordinary words — AT, IN, IS, IT, NO, ME, AM, SO,
 * TO, BY, ID, MY — because "SERVER IN LONDON" must not fly an Indian flag.
 * Those countries are still reachable through their names above.
 */
const BARE_CODES = new Set([
  "NL", "DE", "PL", "FI", "FR", "GB", "UK", "US", "CA", "JP", "KR", "CN", "HK",
  "TW", "SG", "TR", "AE", "IL", "SE", "DK", "IE", "ES", "PT", "CH", "BE", "LU",
  "CZ", "SK", "SI", "HU", "RO", "BG", "RS", "HR", "GR", "CY", "MT", "EE", "LV",
  "LT", "UA", "MD", "RU", "KZ", "UZ", "KG", "GE", "AZ", "AU", "BR", "AR", "MX",
  "ZA", "EG", "IR", "VN", "TH", "PH",
]);

/** UK is the everyday spelling; the flag lives under GB. */
const CODE_ALIASES: Record<string, string> = { UK: "GB" };

const normalize = (name: string): string =>
  name.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ");

function fromName(name: string): FlagInfo | null {
  const cleaned = ` ${normalize(name)} `;
  for (const [code, aliases] of Object.entries(ALIASES)) {
    for (const alias of aliases) {
      const hit =
        alias.length >= 5
          ? cleaned.includes(alias.trim())
          : cleaned.includes(` ${alias.trim()} `);
      if (hit) return { emoji: flagEmoji(code), code };
    }
  }
  // A bare uppercase code, as its own token. Case matters: only "NL" counts,
  // never "nl" inside a hostname-ish label.
  for (const token of name.split(/[^A-Za-z]+/)) {
    if (token.length !== 2 || token !== token.toUpperCase()) continue;
    if (!BARE_CODES.has(token)) continue;
    const code = CODE_ALIASES[token] ?? token;
    return { emoji: flagEmoji(code), code };
  }
  return null;
}

/**
 * Split a server name into its country and the rest of the label.
 *
 * A leading regional-indicator pair is authoritative — the panel already said
 * which country this is, and it is stripped from the label so it can be shown
 * as its own element. Otherwise we infer from a country name or a bare ISO
 * code, and leave the label untouched.
 */
export function readServerName(name: string): ServerName {
  const cps = [...name.trim()];
  const [a, b] = [cps[0]?.codePointAt(0), cps[1]?.codePointAt(0)];
  if (isRegionalIndicator(a) && isRegionalIndicator(b)) {
    const code =
      String.fromCharCode(A + (a - RI_FIRST)) +
      String.fromCharCode(A + (b - RI_FIRST));
    const label = cps.slice(2).join("").replace(/^[\s\-–—·|,]+/, "").trim();
    return {
      flag: { emoji: cps.slice(0, 2).join(""), code },
      // an entry named nothing but its flag keeps the flag as its label
      label: label || name.trim(),
    };
  }
  return { flag: fromName(name), label: name.trim() };
}
