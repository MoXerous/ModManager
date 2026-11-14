#!/usr/bin/env node
/**
 * check-i18n.mjs — يفحص تكافؤ ملفات الترجمة ويمنع مفتاحًا مكتوبًا بيد.
 *
 * القواعد (المصدرُ فيها i18next v26: `Intl.PluralRules` وفئات JSON v4):
 *
 *  أخطاء (تُفشل البناء):
 *   E1  مفتاحٌ في `en.json` بلا مقابلٍ عربي.
 *   E2  مفتاحٌ يستعمله الكودُ غيرُ موجود في `en.json` (خطأٌ مطبعيٌ أو ترجمةٌ ناقصة).
 *   E3  مجموعةُ جمعٍ عربيةٌ ناقصة: بعضُ فئاتِ العربية بلا مقابل.
 *
 *  تحذيرات (لا تُفشل البناء):
 *   W1  مفتاحٌ عربيٌّ بلا مقابلٍ إنجليزي (ت-ra последовательно).
 *   W2  مفتاحٌ إنجليزيٌّ لا يستعمله الكود (ميتٌ أو نسيان).
 *   W3  لاحقةُ جمعٍ يتيمة: أصلُها غيرُ موجود في أيّ من الملفَّين.
 *
 * صيغُ الجمع العربية الستّ (`_zero _one _two _few _many _other`) **مقبولةٌ
 * كمقابلٍ للمفتاح الأساسي** — فوجودُ `key_one` بدل `key` ليس نقصًا.
 *
 *   node scripts/check-i18n.mjs [--quiet]
 */
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const EN_FILE = join(ROOT, "src/i18n/en.json");
const AR_FILE = join(ROOT, "src/i18n/ar.json");
const SRC_DIR = join(ROOT, "src");
const QUIET = process.argv.includes("--quiet");

const CATEGORIES = new Set(new Intl.PluralRules("ar").resolvedOptions().pluralCategories);
const CATEGORY_RE = /_(zero|one|two|few|many|other)$/;

const readJson = (p) => JSON.parse(readFileSync(p, "utf8"));
/** الملفات مسطّحة: المفاتيح أسماءٌ كاملةٌ بنقاطٍ، لا أشجار. */
const keysOf = (o) => Object.keys(o);

const en = readJson(EN_FILE);
const ar = readJson(AR_FILE);
const enKeys = new Set(keysOf(en));
const arKeys = new Set(keysOf(ar));

/** الأصول: مفتاحٌ بلا لاحقةِ جمع. */
const baseOf = (k) => k.replace(CATEGORY_RE, "");
const isPluralForm = (k) => CATEGORY_RE.test(k);
/** هل للمفتاح مجموعةُ جمعٍ في هذه اللغة؟ */
const pluralGroup = (keys, base) => {
  const out = new Set();
  for (const k of keys) if (baseOf(k) === base && isPluralForm(k)) out.add(k.slice(base.length + 1));
  return out;
};
const enPluralBases = new Set();
for (const k of enKeys) if (isPluralForm(k)) enPluralBases.add(baseOf(k));

const errors = [];
const warnings = [];

/** counterparts(o) — ما يقابل مفتاحًا أساسيًّا في هذه اللغة. */
const counterparts = (keys, base) => {
  if (keys.has(base)) return { ok: true, via: base };
  const forms = pluralGroup(keys, base);
  if (forms.size === 0) return { ok: false, via: null };
  const need = CATEGORIES;
  const missing = [...need].filter((c) => !forms.has(c));
  return { ok: missing.length === 0, via: [...forms].join(","), missing };
};

// E1: every English key needs an Arabic counterpart.
for (const k of enKeys) {
  if (isPluralForm(k)) continue; // تُفحص ضمن مجموعتها
  const r = counterparts(arKeys, k);
  if (!r.ok) {
    errors.push(
      r.missing
        ? `E3 ar: "${k}" — مجموعةُ الجمع العربية ناقصة: ينقص ${r.missing.join(", ")} (الموجود: ${r.via})`
        : `E1 ar: "${k}" — لا مقابلَ عربيًّا`
    );
  }
}

// W1: Arabic-only keys. A plural form whose base exists in English is not
// "Arabic-only": English may ship only the base form (i18next falls back to
// it) while Arabic needs all six of its categories.
for (const k of arKeys) {
  if (enKeys.has(k)) continue;
  if (isPluralForm(k) && (enKeys.has(baseOf(k)) || enPluralBases.has(baseOf(k)))) continue;
  if (!isPluralForm(k) && enPluralBases.has(k)) continue;
  warnings.push(`W1 ar: "${k}" — مفتاحٌ عربيٌّ بلا مقابلٍ إنجليزي`);
}

// W3: orphan plural suffixes.
for (const k of arKeys) {
  if (!isPluralForm(k)) continue;
  const base = baseOf(k);
  if (!enKeys.has(base) && !arKeys.has(base) && !enPluralBases.has(base)) {
    warnings.push(`W3 ar: "${k}" — لاحقةُ جمعٍ يتيمة: الأصل "${base}" غيرُ موجود`);
  }
}

// E2: keys used in the code must exist in en.json.
const walk = (dir, out = []) => {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (/\.(ts|tsx)$/.test(e.name)) out.push(p);
  }
  return out;
};
const used = new Map(); // key -> file  (مفاتيحُ مستعملة فعليًّا، بأيّ طريقة)
const tCalled = new Map(); // key -> file (مفاتيحُ نداءٍ مباشر t('...'))
const dynamicPrefixes = new Set(); // مثل "error." و"provider."
if (existsSync(SRC_DIR)) {
  for (const file of walk(SRC_DIR)) {
    const text = readFileSync(file, "utf8");
    const rel = file.replace(ROOT + "\\", "").replace(ROOT + "/", "");
    // كلُّ نصٍّ حرفيّ في الملف: يلتقط جداولَ المفاتيح (error.* مثلًا) لا النداءَ فقط
    for (const m of text.matchAll(/"([A-Za-z0-9_.]+)"|'([A-Za-z0-9_.]+)'/g)) {
      const key = m[1] || m[2];
      if (enKeys.has(key) && !used.has(key)) used.set(key, rel);
    }
    // نداءٌ مباشر: يجبُ أن يكون المفتاحُ موجودًا في en.json (كشفُ الخطأ المطبعي)
    for (const m of text.matchAll(/\bt\(\s*["']([A-Za-z0-9_.]+)["']/g)) {
      const key = m[1];
      if (!tCalled.has(key)) tCalled.set(key, rel);
    }
    // مفاتيحُ مركَّبةٌ ديناميكيًّا: t(`error.${code}`) — لا يمكن كشفها إحصائيًّا،
    // فمفاتيحُها النطاقُ مُعفىٌ من تحذير «غير مستعمل».
    for (const m of text.matchAll(/\bt\(\s*`([^`$]*)\$\{/g)) {
      dynamicPrefixes.add(m[1]);
    }
  }
}
for (const [key, file] of tCalled) {
  if (!enKeys.has(key)) errors.push(`E2 code: "${key}" (${file}) — غيرُ موجود في en.json`);
  else if (!arKeys.has(key) && !counterparts(arKeys, key).ok) errors.push(`E2 ar: "${key}" (${file}) — لا مقابلَ عربيًّا`);
}
// W2: English keys the code never uses. Keys under a dynamically composed
// prefix (t(`error.${code}`)) are exempt — they are used, just not statically.
for (const k of enKeys) {
  if (used.has(k)) continue;
  if (enPluralBases.has(baseOf(k))) continue; // صورُ الجمع تُستعمل عبر أصلها
  if ([...dynamicPrefixes].some((p) => p && k.startsWith(p))) continue;
  warnings.push(`W2 en: "${k}" — لا يستعمله الكود`);
}

// ---- التقرير ----
if (!QUIET) {
  console.log(`i18n: en=${enKeys.size} ar=${arKeys.size} used=${used.size}`);
  if (warnings.length) {
    console.log(`\nتحذيرات (${warnings.length}):`);
    for (const w of warnings) console.log(`  ${w}`);
  }
  if (errors.length) {
    console.log(`\nأخطاء (${errors.length}):`);
    for (const e of errors) console.log(`  ${e}`);
  }
}
if (errors.length) {
  console.error(`\ni18n: ${errors.length} error(s) — the locale files are out of sync`);
  process.exit(1);
}
console.log(`i18n: ok — ${errors.length} errors, ${warnings.length} warnings`);
