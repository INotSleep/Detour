// Word forms for counts.

/** Russian word form for a count: one (1, 21), few (2-4, 22-24), many (5-20, 25...). */
export function ruForm(n: number, one: string, few: string, many: string) {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

/** English word form for a count. */
export const enForm = (n: number, one: string, other: string) => (n === 1 ? one : other);
