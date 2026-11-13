#!/usr/bin/env node
/**
 * build-parity-matrix.mjs — يقيس **قابلية التتبّع** بين مواصفات
 * `docs/spec/features.md` (F-xxx) والشجرة، ويكتب `docs/spec/parity.md`.
 *
 * بلا اعتماديات خارجية.
 *
 *   node scripts/build-parity-matrix.mjs [--check]
 *
 * `--check` لا يكتب ملفًا، بل يخرج برمز خطأ إن كان `docs/spec/parity.md`
 * المُولَّد مختلفًا عن الموجود (بوابةٌ لـ CI).
 *
 * ⚠ الدلالة: هذا قياسُ **أثرٍ قابلٍ للتتبّع**، لا إثباتُ تنفيذٍ أو غيابِه.
 *    المعرّفُ الغائب من الكود لا يعني أن السلوك غير موجود؛ يعني أن الدليل
 *    على وجوده غيرٌ مُسمّى. حالةُ التنفيذ الحقيقية موثَّقة في `docs/todo.md`.
 */
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "..");
const SPEC = path.join(ROOT, "docs/spec/features.md");
const OUT = path.join(ROOT, "docs/spec/parity.md");
const CHECK = process.argv.includes("--check");

const TEST_PATH = /^core\/tests\/|^cli\/tests\/|^tests\/|\/tests?\/|_tests\.rs$|\.test\.[cm]?[jt]sx?$|\/tests\//;
const PROD_PATH = /^core\/src\/|^cli\/src\/|^src-tauri\/src\/|^src\//;
const DOC_PATH = /^docs\/|\.md$/;
const INFRA_PATH = /^\.github\/|^packaging\/|^\.cargo\//;
/**
 * مستنداتُ المواصفات نفسها ليست دليلًا: `features.md` هو المصدرُ المُقارَن،
 * و`parity.md` مخرَجٌ دائريٌّ يذكر كلَّ معرّف، و`todo.md` ادّعاءاتٌ لا شواهد.
 * تُستثنى من جمع الأدلّة كي لا يقرأ القياسُ نفسه دليلًا على نفسه.
 *
 * وهذا الملفُ نفسُه مستثنًى: أمثلتُه في التعليقات تحمل أرقامًا شبيهةً بالمعرّفات،
 * فلو سُوحِ به لكان دليلًا على نفسه.
 */
const SELF_REFERENTIAL = /^docs\/spec\/|^docs\/todo\.md$|^docs\/baseline|^docs\/superpowers\/|^scripts\/build-parity-matrix\.mjs$/;

/** دور الملف في إثبات الأثر. */
function role(rel) {
  const p = rel.replace(/\\/g, "/");
  if (TEST_PATH.test(p)) return "test";
  if (PROD_PATH.test(p)) return "code";
  if (DOC_PATH.test(p)) return "doc";
  if (INFRA_PATH.test(p)) return "infra";
  return "other";
}

/** استخراج المعرّفات من أسماء الاختبارات: spec_fNNN_… · test_fNNN_… · fNNN_…
 *  ملاحظة: لا نستخدم `\b` بعد الأرقام لأن `_` حرفٌ كلمةٍ في JS فلا وجودَ لحدٍّ
 *  كلمةٍ بين الرقم والشرطة السفلية. والأمثلةُ هنا بلا أرقامٍ حقيقية كي لا
 *  يتحوّل التعليقُ إلى دليلٍ على وجود. */
const ID_IN_IDENT = /(?:spec|test)[_-]?f(\d{3})(?![0-9])/gi;
const ID_LOOSE_IN_IDENT = /f(\d{3})_/gi;
/** سطرٌ carries اسمَ اختبار: #[test] · fn spec_… · def test_… · it(… */
const TEST_LINE = /#\[test\]|#\[tokio::test\]|\bfn\s+(spec|test)_|\bdef\s+test_|\bit\(|\bit\.|test\(|describe\(/;

function parseFeatures(md) {
  const rows = [];
  for (const line of md.split(/\r?\n/)) {
    if (!/^\|\s*\*\*F-\d{3}\*\*/.test(line)) continue;
    const cells = line.split("|").slice(1, -1).map((c) => c.trim());
    const id = (cells[0] || "").replace(/\*/g, "").trim();
    if (!/^F-\d{3}$/.test(id)) continue;
    rows.push({
      id,
      name: cells[1] || "",
      domain: cells[3] || "",
      decision: (cells[5] || "").replace(/\*/g, "").trim(),
      phase: cells[6] || "",
    });
  }
  return rows;
}

function gitFiles() {
  const out = execFileSync("git", ["ls-files"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  return out.split(/\r?\n/).filter(Boolean);
}

async function collect(files) {
  const ev = new Map(); // id -> { testId:Set, testMention:Set, code:Set, doc:Set, infra:Set }
  const bucket = (id) => {
    if (!ev.has(id)) ev.set(id, { testId: new Set(), testMention: new Set(), code: new Set(), doc: new Set(), infra: new Set() });
    return ev.get(id);
  };
  for (const rel of files) {
    if (SELF_REFERENTIAL.test(rel.replace(/\\/g, "/"))) continue;
    let text;
    try {
      text = await readFile(path.join(ROOT, rel), "utf8");
    } catch {
      continue;
    }
    const r = role(rel);
    const literal = new Set(text.match(/F-\d{3}/g) || []);
    // الملف مذكورٌ حرفيًّا: صنّفه بحسب دوره
    for (const raw of literal) {
      const b = bucket(raw);
      if (r === "test") b.testMention.add(rel);
      else if (r === "code") b.code.add(rel);
      else if (r === "doc") b.doc.add(rel);
      else if (r === "infra") b.infra.add(rel);
    }
    // المعرّف داخل اسم اختبار (وحدات الاختبار المضمّنة داخل ملفات الإنتاج تُحتسب)
    const lines = text.split(/\r?\n/);
    for (let i = 0; i < lines.length; i += 1) {
      const line = lines[i];
      const ids = new Set();
      for (const re of [ID_IN_IDENT, ID_LOOSE_IN_IDENT]) {
        re.lastIndex = 0;
        let m;
        while ((m = re.exec(line)) !== null) ids.add("F-" + m[1]);
      }
      if (ids.size === 0) continue;
      const isTestLine = TEST_LINE.test(line) || (i > 0 && TEST_LINE.test(lines[i - 1]));
      for (const id of ids) {
        if (isTestLine) bucket(id).testId.add(rel);
        else if (r === "code") bucket(id).code.add(rel);
        else bucket(id).testMention.add(rel);
      }
    }
  }
  return ev;
}

const TIER = {
  TESTED: "مُختبَر بمعرّف المواصفة",
  CODE: "مُنفَّذ ومُشار إليه في الكود",
  TEST_ONLY: "مذكور في الاختبارات (بلا معرّف في الاسم)",
  DOC_ONLY: "إشارة في الوثائق فقط",
  NONE: "بلا أثر",
};

function tierOf(row, e) {
  if (row.decision === "Drop") return "متروك";
  if (row.decision === "Defer") return "مؤجَّل";
  if (!e) return TIER.NONE;
  if (e.testId.size) return TIER.TESTED;
  if (e.code.size) return TIER.CODE;
  if (e.testMention.size) return TIER.TEST_ONLY;
  if (e.doc.size || e.infra.size) return TIER.DOC_ONLY;
  return TIER.NONE;
}

function evidenceOf(e) {
  if (!e) return "—";
  const pick = (s) => [...s].slice(0, 2).join("، ") + (s.size > 2 ? ` (+${s.size - 2})` : "");
  const parts = [];
  if (e.testId.size) parts.push(`اختبار بمعرّف: ${pick(e.testId)}`);
  if (e.code.size) parts.push(`كود: ${pick(e.code)}`);
  if (e.testMention.size) parts.push(`اختبار (ذكر): ${pick(e.testMention)}`);
  if (e.doc.size) parts.push(`وثائق: ${pick(e.doc)}`);
  if (e.infra.size) parts.push(`بنية: ${pick(e.infra)}`);
  return parts.length ? parts.join(" · ") : "—";
}

const md = await readFile(SPEC, "utf8");
const rows = parseFeatures(md);
const evidence = await collect(gitFiles());
const items = rows.map((r) => {
  const e = evidence.get(r.id) || null;
  return { ...r, e, tier: tierOf(r, e) };
});

const keep = items.filter((r) => r.decision === "Keep");
const counts = {};
for (const i of items) counts[i.tier] = (counts[i.tier] || 0) + 1;
const keepTiers = {};
for (const k of keep) keepTiers[k.tier] = (keepTiers[k.tier] || 0) + 1;

const byPhase = {};
for (const k of keep) {
  const p = k.phase || "—";
  byPhase[p] = byPhase[p] || { total: 0, tested: 0, code: 0, other: 0 };
  byPhase[p].total++;
  if (k.tier === TIER.TESTED) byPhase[p].tested++;
  else if (k.tier === TIER.CODE) byPhase[p].code++;
  else byPhase[p].other++;
}

const untraceable = keep.filter((k) => k.tier === TIER.NONE || k.tier === TIER.DOC_ONLY);

const L = [];
L.push("# مصفوفة قابلية التتبّع للمواصفات — ModManager");
L.push("");
L.push("> **مولَّدة آليًّا**: `node scripts/build-parity-matrix.mjs` — تقرأ `docs/spec/features.md`");
L.push("> وتمسح الشجرة عن حضور كل معرّف `F-xxx`. بلا اعتماديات خارجية.");
L.push("");
L.push("## ما الذي تقيسه هذه المصفوفة");
L.push("");
L.push("قياسُ **قابلية التتبّع**: هل يحمل أثرُ السلوك معرّفَ مواصفته في مكانٍ خاضعٍ للبحث؟");
L.push("هذا عُرفُ المستودع نفسه (`AGENTS.md` §6: «أسماءُ الاختبارات تحمل معرّف المواصفة»).");
L.push("");
L.push("> ⚠ **ليس قياسَ تنفيذ**: غيابُ المعرّف من الكود **لا يعني** غيابَ السلوك؛ يعني أن الدليل");
L.push("> عليه غيرُ مُسمّى. حالةُ التنفيذ موثَّقة في `docs/todo.md`.");
L.push("");
L.push("**ما هو مستثنى من جمع الأدلّة:** `docs/spec/` (المصدرُ المُقارَن والمخرَجُ الدائريّ) · `docs/todo.md` (ادّعاءات) · `docs/baseline*` و`docs/superpowers/` · وهذا السكربتُ نفسُه. لولا ذلك لقرأ القياسُ نفسه دليلًا على نفسه — وقد حدث ذلك فعلًا مرّتين في أثناء التطوير.");
L.push("");
L.push("## الملخّص");
L.push("");
L.push(`- **${items.length}** بندًا في المواصفات · **${keep.length}** منها \`Keep\` · ${items.filter((i) => i.decision === "Defer").length} \`Defer\` · ${items.filter((i) => i.decision === "Drop").length} \`Drop\`.`);
L.push("");
L.push("| الأثر | بنود `Keep` | بنود كلُّها |");
L.push("|---|---|---|");
for (const t of [TIER.TESTED, TIER.CODE, TIER.TEST_ONLY, TIER.DOC_ONLY, TIER.NONE]) {
  L.push(`| ${t} | ${keepTiers[t] || 0} | ${counts[t] || 0} |`);
}
L.push("");
L.push(`**${keepTiers[TIER.TESTED] || 0} من ${keep.length}** بندًا \`Keep\` له اختبارٌ يحمل معرّفه · **${keepTiers[TIER.CODE] || 0}** مذكورٌ في كود الإنتاج · **${untraceable.length}** بلا أثرٍ قابلٍ للتتبّع.`);
L.push("");
L.push("## حسب المرحلة");
L.push("");
L.push("| المرحلة | بنود Keep | مُختبَر بمعرّف | مُشار إليه في الكود | بلا أثر |");
L.push("|---|---|---|---|---|");
for (const [p, v] of Object.entries(byPhase).sort()) {
  L.push(`| ${p} | ${v.total} | ${v.tested} | ${v.code} | ${v.other} |`);
}
L.push("");
L.push("## بنود `Keep` بلا أثر قابلٍ للتتبّع — قائمة hygiene");
L.push("");
L.push("> هذه ليست قائمةَ «غير مُنفَّذ». إنها قائمةُ **أدلّةٍ بلا أسماء** — إما اختبارٌ يحمل معرّفًا غير مطابق، أو تنفيذٌ بلا إشارة.");
L.push("");
L.push("| المعرّف | البند | المجال | المرحلة |");
L.push("|---|---|---|---|");
for (const r of untraceable) L.push(`| **${r.id}** | ${r.name} | ${r.domain} | ${r.phase} |`);
L.push("");
L.push("## التفاصيل");
L.push("");
let domain = null;
for (const r of items) {
  if (r.domain !== domain) {
    domain = r.domain;
    L.push(`### ${domain}`);
    L.push("");
    L.push("| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |");
    L.push("|---|---|---|---|---|---|");
  }
  L.push(`| **${r.id}** | ${r.name} | ${r.decision} | ${r.phase} | ${r.tier} | ${evidenceOf(r.e)} |`);
}
L.push("");
L.push("---");
L.push("");
L.push("**قراءة الأرقام:** `مُختبَر بمعرّف المواصفة` = المعرّف داخل اسم اختبار (أو عنوانه) · `مُنفَّذ ومُشار إليه في الكود` = المعرّف حرفيًّا في كود إنتاج · `مذكور في الاختبارات` = حرفيًّا في ملف اختبار بلا معرّف في الاسم · `إشارة في الوثائق فقط` · `بلا أثر` = لا ذكر على الإطلاق.");
L.push("");

const text = L.join("\n");
if (CHECK) {
  let current = "";
  try {
    current = await readFile(OUT, "utf8");
  } catch {
    /* لا ملف بعد */
  }
  if (current !== text) {
    console.error("parity: out of date — run `node scripts/build-parity-matrix.mjs`");
    process.exit(1);
  }
  console.log("parity: up to date");
} else {
  await writeFile(OUT, text, "utf8");
  console.log(
    `parity: wrote ${path.relative(ROOT, OUT)} — specs=${items.length} keep=${keep.length} tested=${keepTiers[TIER.TESTED] || 0} code=${keepTiers[TIER.CODE] || 0} untraceable=${untraceable.length}`
  );
}
