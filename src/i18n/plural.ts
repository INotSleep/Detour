// Word forms for counts, by the CLDR rules built into the browser (Intl.PluralRules).
// Each language has its own function that asks for exactly the forms that language has,
// so tsc says when a translation leaves one out.

type Cat = Intl.LDMLPluralRule;
const rules = new Map<string, Intl.PluralRules>();

function form(locale: string, n: number, forms: Partial<Record<Cat, string>>, fallback: string): string {
  let r = rules.get(locale);
  if (!r) rules.set(locale, (r = new Intl.PluralRules(locale)));
  return forms[r.select(n)] ?? fallback;
}

type OneOther = { one: string; other: string };
type Slavic = { one: string; few: string; many: string };

/** English: one (1), other. */
export const en = (n: number, f: OneOther) => form("en", n, f, f.other);
/** Russian: one (1, 21), few (2-4, 22-24), many (5-20, 25...); fractions take few. */
export const ru = (n: number, f: Slavic) => form("ru", n, f, f.few);
/** Ukrainian: as Russian. */
export const uk = (n: number, f: Slavic) => form("uk", n, f, f.few);
/** German: one, other. */
export const de = (n: number, f: OneOther) => form("de", n, f, f.other);
/** Spanish: one, other (many is for "de millones", which counts here never reach). */
export const es = (n: number, f: OneOther) => form("es", n, f, f.other);
/** Brazilian Portuguese: one (0 and 1), other. */
export const pt = (n: number, f: OneOther) => form("pt", n, f, f.other);
/** Chinese has one form for every count. */
export const zh = (_n: number, f: { other: string }) => f.other;
