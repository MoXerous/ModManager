import { type Numerals, getPreferences } from '../app/preferences';

const INTL_LOCALE: Record<Numerals, string> = { latn: 'en-US', arab: 'ar-EG' };

export function formatNumber(value: number, numerals: Numerals = getPreferences().numerals): string {
  return new Intl.NumberFormat(INTL_LOCALE[numerals]).format(value);
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const;

/**
 * Matches the boards' verbose form: "36 B", "0.4 MB", "212 MB", "1.2 GB".
 * The boards also use a compact table form ("860K", "12.4M"); that lands with
 * the Browse table in 5c.
 */
export function formatBytes(bytes: number, numerals: Numerals = getPreferences().numerals): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const amount = unit === 0 ? String(Math.round(value)) : value.toFixed(1).replace(/\.0$/, '');
  return `${formatNumber(Number(amount), numerals)} ${BYTE_UNITS[unit]}`;
}