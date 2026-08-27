const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Guards a lookup key that maps to a `uuid` column. A non-UUID string would
 * make Postgres throw `22P02`; callers use this to return "not found" instead.
 */
export function isUuid(value: string): boolean {
  return UUID_RE.test(value);
}
