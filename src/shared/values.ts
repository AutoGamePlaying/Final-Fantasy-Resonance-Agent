/**
 * Reading the loosely shaped JSON that the bridge and the game machine send, with empty-list
 * and empty-dict semantics matching the reference behaviour. Use these instead of re-deriving
 * the rules in each module.
 */

export type Json = Record<string, unknown>;

export function isRecord(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Python truthiness: None, False, 0, "", [] and {} are false. */
export function truthy(value: unknown): boolean {
  if (Array.isArray(value)) return value.length > 0;
  if (isRecord(value)) return Object.keys(value).length > 0;
  return Boolean(value);
}

/** `value or {}` for a dict: the object itself, or an empty one for anything else. */
export function record(value: unknown): Json {
  return isRecord(value) ? value : {};
}

/** `value or []` for a list: the array itself, or an empty one for anything else. */
export function list(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

/** A list of dicts; entries that are not objects are left out. */
export function records(value: unknown): Json[] {
  return list(value).filter(isRecord);
}

export function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

export function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}
